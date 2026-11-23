/** Quality: the L3 chain, SPC, alarms/OCAP, Q-time, RMS and yield. */
import { Text, View } from "nsh";
import {
  analysis,
  copyTraceEvent,
  drill,
  loadTrace,
  spc,
  spcSource,
  trace,
  traceEvent,
  selectedWafer,
  detail,
} from "../store";
import { durationText, type SpcBucket } from "../model";
import { Badge, Button, Card, Divider, Dot, Kv, Small, Strong, p, BODY, SMALL, TINY } from "../components/ui";

function traceCardBody() {
  const loaded = trace();
  if (loaded === null) {
    return <Small text="告警、Q-time 违规或 L3 定位后，这条链会出现在这里。" />;
  }
  const outbox = loaded.outbox;
  const tables = Object.entries(loaded.greptime)
    .map(([table, rows]) => `${table} ${rows.length} 行`)
    .join(" · ");
  const outboxText =
    outbox === null
      ? "无 outbox 行"
      : outbox.published_at === null
        ? `待投递 · 尝试 ${outbox.attempts} 次`
        : `已投递 ${outbox.published_at}`;
  return (
    <View style={{ gap: 4 }}>
      <Kv key="event_id" value={loaded.event_id} />
      <Kv key="outbox" value={outboxText} />
      <Kv key="GreptimeDB" value={tables === "" ? "该事件不产指标行" : tables} />
      <View style={{ flexDirection: "row" }}>
        <Button label="复制 event_id" onPress={copyTraceEvent} />
      </View>
    </View>
  );
}

function spcChart() {
  const buckets = spc();
  if (buckets.length === 0) {
    return <Small text="等待量测样本（S4 完成后出现）" />;
  }
  const last = buckets[buckets.length - 1];
  const ucl = last?.ucl ?? 103;
  const lcl = last?.lcl ?? 97;
  let low = Math.min(lcl, ...buckets.map((bucket) => bucket.min_value));
  let high = Math.max(ucl, ...buckets.map((bucket) => bucket.max_value));
  low -= 0.3;
  high += 0.3;
  const span = Math.max(high - low, 0.1);
  const outliers = buckets.filter((bucket) => bucket.avg_value > bucket.ucl || bucket.avg_value < bucket.lcl);
  return (
    <View style={{ gap: 8 }}>
      <View
        style={{
          height: 96,
          flexDirection: "row",
          alignItems: "flex-end",
          gap: 2,
          padding: 4,
          borderRadius: 8,
          backgroundColor: p().surfaceAlt,
        }}
      >
        {buckets.map((bucket: SpcBucket) => {
          const height = Math.max(2, Math.min(86, ((bucket.avg_value - low) / span) * 86));
          const outside = bucket.avg_value > bucket.ucl || bucket.avg_value < bucket.lcl;
          return (
            <View
              style={{
                flex: 1,
                minWidth: 2,
                height,
                borderRadius: 1.5,
                backgroundColor: outside ? p().bad : p().good,
              }}
            />
          );
        })}
      </View>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Text style={{ fontSize: SMALL, color: p().muted }}>{`UCL ${ucl.toFixed(1)}`}</Text>
        <View style={{ flex: 1 }} />
        <Text style={{ fontSize: SMALL, color: p().text }}>{`最近 ${last?.avg_value.toFixed(2)} nm`}</Text>
        <View style={{ flex: 1 }} />
        <Text style={{ fontSize: SMALL, color: p().muted }}>{`LCL ${lcl.toFixed(1)}`}</Text>
      </View>
      {outliers
        .slice()
        .reverse()
        .slice(0, 5)
        .map((bucket) => (
          <View
            onPress={() => drill({ kind: "tool", equipment_id: bucket.equipment_id })}
            style={{
              flexDirection: "row",
              alignItems: "center",
              gap: 8,
              paddingHorizontal: 6,
              paddingVertical: 4,
              borderRadius: 5,
              backgroundColor: p().surfaceAlt,
            }}
          >
            <Dot color={p().bad} />
            <Text style={{ fontSize: SMALL, color: p().muted }}>{`${bucket.equipment_id} · ${bucket.ts}`}</Text>
            <View style={{ flex: 1 }} />
            <Text style={{ fontSize: SMALL, color: p().muted }}>
              {`${bucket.avg_value.toFixed(2)} nm · ${bucket.lcl.toFixed(1)}–${bucket.ucl.toFixed(1)}`}
            </Text>
          </View>
        ))}
      <Text style={{ fontSize: TINY, color: p().muted }}>
        {`来源 ${spcSource()} · ${buckets.length} 个时间桶 · 规格 ${lcl.toFixed(0)}–${ucl.toFixed(0)} nm`}
      </Text>
    </View>
  );
}

function alarmsBody() {
  const current = analysis();
  if (current === null) return <Small text="读取 analytics…" />;
  if (current.alarms.length === 0) {
    return <Small text="无设备告警：equipment_down 会带着 event_id 出现在这里" />;
  }
  return (
    <View style={{ gap: 4 }}>
      {current.alarms
        .slice()
        .reverse()
        .slice(0, 6)
        .map((alarm) => (
          <View
            onPress={() => void loadTrace(alarm.event_id)}
            style={{
              flexDirection: "row",
              alignItems: "center",
              gap: 8,
              paddingHorizontal: 6,
              paddingVertical: 4,
              borderRadius: 5,
              backgroundColor: p().surfaceAlt,
            }}
          >
            <Dot color={alarm.kind === "down" ? p().bad : p().good} />
            <Text style={{ fontSize: SMALL, color: p().text }}>{`${alarm.equipment_id} · ${alarm.reason}`}</Text>
            <View style={{ flex: 1 }} />
            <Button label="机台" onPress={() => drill({ kind: "tool", equipment_id: alarm.equipment_id })} />
          </View>
        ))}
    </View>
  );
}

function qtimeBody() {
  const current = analysis();
  if (current === null) return <Small text="读取 analytics…" />;
  return (
    <View style={{ gap: 4 }}>
      <Text style={{ fontSize: SMALL, color: p().muted }}>
        {`上限 ${current.qtime_limits
          .map((limit) => `S${limit.from_step}→S${limit.to_step} ≤ ${Math.round(limit.limit_ms / 1000)}s`)
          .join(" · ")}`}
      </Text>
      {current.qtime_violations.length === 0 ? (
        <Small text="无违规：所有跨工序等待都在上限内" />
      ) : (
        current.qtime_violations.slice(0, 5).map((violation) => (
          <View
            onPress={() => void loadTrace(violation.event_id)}
            style={{
              flexDirection: "row",
              alignItems: "center",
              gap: 8,
              paddingHorizontal: 6,
              paddingVertical: 4,
              borderRadius: 5,
              backgroundColor: p().surfaceAlt,
            }}
          >
            <Dot color={p().warn} />
            <Text style={{ fontSize: SMALL, color: p().text }}>
              {`${violation.lot_id} · S${violation.from_step}→S${violation.to_step} · ${Math.round(
                violation.elapsed_ms / 1000,
              )}s > ${Math.round(violation.limit_ms / 1000)}s · ${violation.wafer_ids.length} 片`}
            </Text>
            <View style={{ flex: 1 }} />
            <Button label="机台" onPress={() => drill({ kind: "tool", equipment_id: violation.equipment_id })} />
          </View>
        ))
      )}
      <Small
        text={Object.entries(
          current.qtime_violations.reduce<Record<string, number>>((counts, violation) => {
            const key = `${violation.from_step}→${violation.to_step}`;
            counts[key] = (counts[key] ?? 0) + 1;
            return counts;
          }, {}),
        )
          .sort(([left], [right]) => left.localeCompare(right))
          .map(([key, count]) => `${key} × ${count}`)
          .reduce((left, right) => (left === "" ? `跨班次重复模式：${right}` : `${left} · ${right}`), "")}
      />
    </View>
  );
}

function recipesBody() {
  const current = analysis();
  if (current === null) return <Small text="读取 analytics…" />;
  return (
    <View style={{ gap: 4 }}>
      {current.recipes.map((recipe) => (
        <View
          style={{
            flexDirection: "row",
            alignItems: "center",
            gap: 8,
            paddingHorizontal: 6,
            paddingVertical: 4,
            borderRadius: 5,
            backgroundColor: p().surfaceAlt,
          }}
        >
          <Dot color={recipe.mismatched_runs > 0 ? p().bad : recipe.runs > 0 ? p().good : p().muted} />
          <Text style={{ fontSize: SMALL, color: p().text }}>
            {`${recipe.recipe_id} v${recipe.version} · ${recipe.checksum.slice(0, 10)}`}
          </Text>
          <View style={{ flex: 1 }} />
          <Small
            text={
              recipe.mismatched_runs > 0
                ? `${recipe.mismatched_runs} 次不一致`
                : recipe.runs > 0
                  ? `${recipe.runs} 次执行均一致`
                  : "尚未执行"
            }
          />
        </View>
      ))}
    </View>
  );
}

function r2rBody() {
  const buckets = spc();
  if (buckets.length === 0) return <View />;
  const last = buckets[buckets.length - 1];
  if (last === undefined) return <View />;
  const target = (last.ucl + last.lcl) / 2;
  const prior = buckets.length > 1 ? buckets[buckets.length - 2].avg_value : last.avg_value;
  const suggestion = target - last.avg_value;
  return (
    <View style={{ gap: 4 }}>
      <Kv key="规格中心" value={`${target.toFixed(2)} nm`} />
      <Kv key="上次均值" value={`${prior.toFixed(2)} nm`} />
      <Kv key="本次均值" value={`${last.avg_value.toFixed(2)} nm`} />
      <Kv key="建议调整" value={`${suggestion >= 0 ? "+" : ""}${suggestion.toFixed(2)} nm（只读展示）`} />
      <Small text="R2R 只给出调整量，不闭环写回设备；输入全部来自 spc_sample。" />
    </View>
  );
}

function yieldBody() {
  const current = analysis();
  if (current === null) return <Small text="读取 analytics…" />;
  const kpi = current.kpi;
  const wafer = selectedWafer();
  const source = wafer === null ? undefined : detail()?.genealogy.find((entry) => entry.wafer_id === wafer)?.steps.at(-1);
  return (
    <View style={{ gap: 4 }}>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Strong text={`FPY ${((kpi.first_pass_yield ?? 0) * 100).toFixed(1)}%`} />
        <Small text={`报废 ${kpi.wafers_scrapped} / 出现 ${kpi.wafers_created} 片`} />
      </View>
      <Small
        text={`缺陷分类：${kpi.defects.map((defect) => `${defect.class} ${defect.count}`).join(" · ")}`}
      />
      {source === undefined ? null : (
        <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
          <Small text="缺陷来源层" />
          <Button label="定位来源机台" onPress={() => drill({ kind: "tool", equipment_id: source.equipment_id })} />
        </View>
      )}
    </View>
  );
}

export function QualityPanel() {
  return (
    <View style={{ gap: 12 }}>
      <Card title="L3 事件 → outbox → 数据点" source="event_id 可复制 · 三处可核对">
        {traceCardBody()}
      </Card>
      <Card title="SPC · 控制图" source="spc_sample 降采样 · 超限关联机台">
        {spcChart()}
      </Card>
      <Card title="告警与 OCAP 路径" source="equipment_down/up · 可追到 event_id">
        {alarmsBody()}
      </Card>
      <Card title="Q-time · 连续工序上限" source="lot_event.occurred_at + route_step.qtime_ms">
        {qtimeBody()}
      </Card>
      <Card title="配方 · RMS 校验" source="recipe.version/checksum ↔ track_out">
        {recipesBody()}
      </Card>
      <Card title="Run-to-Run · 只读建议" source="spc_sample 均值与规格中心 · 不写回设备">
        {r2rBody()}
      </Card>
      <Card title="缺陷与良率" source="wafer.defect_info / wafer_scrapped">
        {yieldBody()}
      </Card>
    </View>
  );
}
