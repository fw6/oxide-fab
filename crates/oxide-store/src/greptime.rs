//! `GreptimeDB`: the derived time series store.
//!
//! Every write goes through the official HTTP SQL interface, and every table
//! is created with the primary key that includes `event_id`, so re-delivering
//! the same event cannot add a point. The database is disposable: it can be
//! truncated and rebuilt from PostgreSQL's `lot_event`.

use std::collections::BTreeMap;

use oxide_core::metrics::{MetricRow, MetricTable};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum GreptimeError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("greptime: {0}")]
    Server(String),
    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, GreptimeError>;

/// One downsampled SPC bucket as the chart consumes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpcBucket {
    pub ts: String,
    pub equipment_id: String,
    pub param: String,
    pub avg_value: f64,
    pub min_value: f64,
    pub max_value: f64,
    pub ucl: f64,
    pub lcl: f64,
}

#[derive(Clone, Debug)]
pub struct GreptimeDb {
    base: String,
    database: String,
    client: reqwest::Client,
}

impl GreptimeDb {
    pub fn new(base: impl Into<String>, database: impl Into<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_owned(),
            database: database.into(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("http client"),
        }
    }

    pub async fn ping(&self) -> bool {
        self.execute("SELECT 1").await.is_ok()
    }

    /// Run one SQL statement. SELECTs come back as objects keyed by column.
    pub async fn execute(&self, sql: &str) -> Result<Vec<serde_json::Map<String, Value>>> {
        let response = self
            .client
            .post(format!("{}/v1/sql", self.base))
            .query(&[("db", self.database.as_str())])
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(format!("sql={}", urlencode(sql)))
            .send()
            .await?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|e| GreptimeError::Server(format!("HTTP {status}: {e}")))?;
        if let Some(error) = body.get("error").and_then(Value::as_str) {
            return Err(GreptimeError::Server(error.to_owned()));
        }
        if !status.is_success() {
            return Err(GreptimeError::Server(format!("HTTP {status}: {body}")));
        }
        Ok(rows_to_objects(&body))
    }

    /// Create the four tables (and the downsampling flow) if they are absent.
    pub async fn ensure_schema(&self) -> Result<()> {
        for ddl in SCHEMA {
            self.execute(ddl).await?;
        }
        // The flow is not `IF NOT EXISTS` in every version: check first.
        let existing = self
            .execute("SELECT flow_name FROM information_schema.flows")
            .await
            .unwrap_or_default();
        let present = existing.iter().any(|row| {
            row.get("flow_name")
                .and_then(Value::as_str)
                .is_some_and(|name| name == "spc_downsample")
        });
        if !present {
            self.execute(DOWNSAMPLE_FLOW).await?;
        }
        Ok(())
    }

    /// Write rows, one multi-value INSERT per table.
    pub async fn insert_rows(&self, rows: &[MetricRow]) -> Result<usize> {
        let mut by_table: BTreeMap<&'static str, Vec<&MetricRow>> = BTreeMap::new();
        for row in rows {
            by_table.entry(row.table().as_str()).or_default().push(row);
        }
        let mut written = 0;
        for (table, rows) in by_table {
            for chunk in rows.chunks(200) {
                let values: Vec<String> = chunk.iter().map(|row| literal_row(row)).collect();
                let sql = format!(
                    "INSERT INTO {} ({}) VALUES {}",
                    table,
                    columns(table),
                    values.join(", ")
                );
                self.execute(&sql).await?;
                written += chunk.len();
            }
        }
        Ok(written)
    }

    /// Rows per table, as the replay verification compares them.
    pub async fn table_counts(&self) -> Result<BTreeMap<String, i64>> {
        let mut counts = BTreeMap::new();
        for table in MetricTable::all() {
            counts.insert(table.as_str().to_owned(), self.counts_for(table).await?);
        }
        Ok(counts)
    }

    /// A cheap aggregate used to prove a replay produced the same values.
    pub async fn spc_average(&self) -> Result<f64> {
        let rows = self
            .execute("SELECT avg(`value`) AS m FROM spc_sample")
            .await?;
        Ok(rows
            .first()
            .and_then(|row| row.get("m"))
            .and_then(|value| {
                value
                    .as_f64()
                    .or_else(|| value.as_str().and_then(|s| s.parse::<f64>().ok()))
            })
            .unwrap_or(0.0))
    }

    pub async fn truncate_all(&self) -> Result<()> {
        for table in MetricTable::all() {
            self.execute(&format!("TRUNCATE TABLE {}", table.as_str()))
                .await?;
        }
        // The flow's sink is derived from `spc_sample` but lives on its own
        // TTL. Empty it too, or the chart keeps showing the previous run's
        // buckets after a reset or replay.
        self.execute("TRUNCATE TABLE spc_downsample").await?;
        Ok(())
    }

    /// The SPC chart reads the continuous aggregation's sink table.
    pub async fn spc_series(&self, limit: i64) -> Result<Vec<SpcBucket>> {
        let sql = format!(
            "SELECT ts, equipment_id, param, avg_value, min_value, max_value, ucl, lcl
             FROM spc_downsample ORDER BY ts DESC LIMIT {limit}"
        );
        Ok(buckets_from_rows(&self.execute(&sql).await?))
    }

    /// Raw samples, used when the flow has not produced a bucket yet.
    pub async fn raw_spc_series(&self, limit: i64) -> Result<Vec<SpcBucket>> {
        let sql = format!(
            "SELECT ts, equipment_id, param, `value` AS avg_value, `value` AS min_value,
                    `value` AS max_value, ucl, lcl
             FROM spc_sample ORDER BY ts DESC LIMIT {limit}"
        );
        Ok(buckets_from_rows(&self.execute(&sql).await?))
    }

    /// Every row the four derived tables hold for one event, keyed by table.
    /// The values are the columns `GreptimeDB` returns, unchanged.
    pub async fn rows_for_event(
        &self,
        event_id: uuid::Uuid,
    ) -> Result<std::collections::BTreeMap<String, Vec<serde_json::Value>>> {
        let mut rows = std::collections::BTreeMap::new();
        for table in MetricTable::all() {
            let sql = format!(
                "SELECT * FROM {} WHERE event_id = '{}' ORDER BY ts LIMIT 20",
                table.as_str(),
                event_id
            );
            let result = self.execute(&sql).await?;
            if !result.is_empty() {
                rows.insert(
                    table.as_str().to_owned(),
                    result.into_iter().map(serde_json::Value::Object).collect(),
                );
            }
        }
        Ok(rows)
    }

    pub async fn counts_for(&self, table: MetricTable) -> Result<i64> {
        let rows = self
            .execute(&format!("SELECT count(*) AS c FROM {}", table.as_str()))
            .await?;
        Ok(rows
            .first()
            .and_then(|row| row.get("c"))
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_u64().map(|v| v as i64))
                    .or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok()))
            })
            .unwrap_or(0))
    }
}

/// One SPC row as the chart consumes it.
///
/// The flow's sink and the raw sample table are projected onto the same
/// columns by their queries, so both go through this one shape.
fn bucket_from_row(row: &serde_json::Map<String, Value>) -> Option<SpcBucket> {
    Some(SpcBucket {
        ts: timestamp_text(row.get("ts")?)?,
        equipment_id: row.get("equipment_id")?.as_str()?.to_owned(),
        param: row.get("param")?.as_str()?.to_owned(),
        avg_value: number(row.get("avg_value")?)?,
        min_value: number(row.get("min_value")?)?,
        max_value: number(row.get("max_value")?)?,
        ucl: number(row.get("ucl")?)?,
        lcl: number(row.get("lcl")?)?,
    })
}

/// The queries return newest first; the chart reads left to right.
fn buckets_from_rows(rows: &[serde_json::Map<String, Value>]) -> Vec<SpcBucket> {
    let mut buckets: Vec<SpcBucket> = rows.iter().filter_map(bucket_from_row).collect();
    buckets.reverse();
    buckets
}

/// `GreptimeDB` returns `TIMESTAMP` columns as epoch milliseconds; the API hands
/// the dashboard an RFC 3339 string either way.
fn timestamp_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_owned());
    }
    let millis = value.as_i64()?;
    chrono::DateTime::from_timestamp_millis(millis).map(|moment| moment.to_rfc3339())
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse::<f64>().ok()))
}

fn rows_to_objects(body: &Value) -> Vec<serde_json::Map<String, Value>> {
    let mut out = Vec::new();
    if let Some(outputs) = body.get("output").and_then(Value::as_array) {
        for output in outputs {
            let Some(records) = output.get("records") else {
                continue;
            };
            let names: Vec<String> = records
                .get("schema")
                .and_then(|schema| schema.get("column_schemas"))
                .and_then(Value::as_array)
                .map_or_default(|schemas| {
                    schemas
                        .iter()
                        .filter_map(|schema| {
                            schema
                                .get("name")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                        .collect()
                });
            let Some(rows) = records.get("rows").and_then(Value::as_array) else {
                continue;
            };
            for row in rows {
                let Some(values) = row.as_array() else {
                    continue;
                };
                let mut object = serde_json::Map::new();
                for (index, name) in names.iter().enumerate() {
                    object.insert(
                        name.clone(),
                        values.get(index).cloned().unwrap_or(Value::Null),
                    );
                }
                out.push(object);
            }
        }
    }
    out
}

fn urlencode(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(input.len() + 16);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            other => {
                out.push('%');
                out.push(char::from(HEX[usize::from(other >> 4)]));
                out.push(char::from(HEX[usize::from(other & 0x0f)]));
            }
        }
    }
    out
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn timestamp_literal(ms: i64) -> String {
    let datetime = chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_else(chrono::Utc::now)
        .format("%Y-%m-%d %H:%M:%S%.3f");
    format!("'{datetime}'::TIMESTAMP_MS")
}

fn columns(table: &str) -> &'static str {
    match table {
        "equipment_telemetry" => {
            "ts, event_id, equipment_id, lot_id, step_id, param, `value`, unit, quality"
        }
        "gem_event" => "ts, event_id, equipment_id, `stream`, `function`, ceid, alid, payload",
        "spc_sample" => "ts, event_id, equipment_id, param, `value`, ucl, lcl",
        "e116_perf" => "ts, event_id, equipment_id, `state`, duration_ms",
        _ => "",
    }
}

fn literal_row(row: &MetricRow) -> String {
    match row {
        MetricRow::Telemetry(p) => format!(
            "({}, {}, {}, {}, {}, {}, {}, {}, {})",
            timestamp_literal(p.ts_ms),
            quote(&p.event_id.to_string()),
            quote(&p.equipment_id),
            quote(&p.lot_id),
            quote(&p.step_id),
            quote(&p.param),
            p.value,
            quote(&p.unit),
            quote(&p.quality),
        ),
        MetricRow::Gem(p) => format!(
            "({}, {}, {}, {}, {}, {}, {}, {})",
            timestamp_literal(p.ts_ms),
            quote(&p.event_id.to_string()),
            quote(&p.equipment_id),
            p.stream,
            p.function,
            p.ceid,
            p.alid,
            quote(&p.payload),
        ),
        MetricRow::Spc(p) => format!(
            "({}, {}, {}, {}, {}, {}, {})",
            timestamp_literal(p.ts_ms),
            quote(&p.event_id.to_string()),
            quote(&p.equipment_id),
            quote(&p.param),
            p.value,
            p.ucl,
            p.lcl,
        ),
        MetricRow::Perf(p) => format!(
            "({}, {}, {}, {}, {})",
            timestamp_literal(p.ts_ms),
            quote(&p.event_id.to_string()),
            quote(&p.equipment_id),
            quote(&p.state),
            p.duration_ms,
        ),
    }
}

const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS equipment_telemetry (
        ts TIMESTAMP TIME INDEX,
        event_id STRING,
        equipment_id STRING,
        lot_id STRING,
        step_id STRING,
        param STRING,
        `value` DOUBLE,
        unit STRING,
        quality STRING,
        PRIMARY KEY (event_id, param)
     ) WITH (ttl = '30d')",
    "CREATE TABLE IF NOT EXISTS gem_event (
        ts TIMESTAMP TIME INDEX,
        event_id STRING,
        equipment_id STRING,
        `stream` INT,
        `function` INT,
        ceid BIGINT,
        alid BIGINT,
        payload STRING,
        PRIMARY KEY (event_id)
     ) WITH (ttl = '30d')",
    "CREATE TABLE IF NOT EXISTS spc_sample (
        ts TIMESTAMP TIME INDEX,
        event_id STRING,
        equipment_id STRING,
        param STRING,
        `value` DOUBLE,
        ucl DOUBLE,
        lcl DOUBLE,
        PRIMARY KEY (event_id)
     ) WITH (ttl = '30d')",
    "CREATE TABLE IF NOT EXISTS e116_perf (
        ts TIMESTAMP TIME INDEX,
        event_id STRING,
        equipment_id STRING,
        `state` STRING,
        duration_ms BIGINT,
        PRIMARY KEY (event_id)
     ) WITH (ttl = '30d')",
    "CREATE TABLE IF NOT EXISTS spc_downsample (
        ts TIMESTAMP TIME INDEX,
        equipment_id STRING,
        param STRING,
        avg_value DOUBLE,
        min_value DOUBLE,
        max_value DOUBLE,
        ucl DOUBLE,
        lcl DOUBLE,
        PRIMARY KEY (equipment_id, param)
     ) WITH (ttl = '7d')",
];

const DOWNSAMPLE_FLOW: &str = "CREATE FLOW spc_downsample
    SINK TO spc_downsample
    AS SELECT
        equipment_id,
        param,
        date_bin('10 seconds'::INTERVAL, ts) AS ts,
        avg(`value`) AS avg_value,
        min(value) AS min_value,
        max(value) AS max_value,
        max(ucl) AS ucl,
        min(lcl) AS lcl
    FROM spc_sample
    GROUP BY equipment_id, param, date_bin('10 seconds'::INTERVAL, ts)";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_becomes_one_insert_literal() {
        let row = MetricRow::Spc(oxide_core::metrics::SpcPoint {
            ts_ms: 1_700_000_000_123,
            event_id: uuid::Uuid::nil(),
            equipment_id: "MET-01".to_owned(),
            param: "metrology_cd_nm".to_owned(),
            value: 100.5,
            ucl: 103.0,
            lcl: 97.0,
        });
        let literal = literal_row(&row);
        assert!(literal.contains("'2023-11-14 22:13:20.123'::TIMESTAMP_MS"));
        assert!(literal.contains("'MET-01'"));
        assert!(literal.contains("100.5"));
    }

    #[test]
    fn quotes_are_escaped() {
        assert_eq!(quote("it's"), "'it''s'");
    }

    #[test]
    fn the_form_body_is_url_encoded() {
        assert_eq!(urlencode("SELECT 1"), "SELECT+1");
        assert_eq!(urlencode("a'b"), "a%27b");
    }
}
