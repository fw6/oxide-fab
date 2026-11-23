/** Cross-cutting: the L3 chain, monitor, outbox, KPI, drills, demo and SECS. */
import { Text, View } from "nsh";
import {
  analysis,
  backup,
  backend,
  chaos,
  copyTraceEvent,
  demo,
  demoRunning,
  dense,
  eventsTotal,
  exportChaos,
  exportLot,
  exportScene,
  greptimeOk,
  loadMonitor,
  monitor,
  outboxPending,
  outboxPublished,
  publishedEvents,
  reduceMotion,
  restore,
  secs,
  sim,
  startBackend,
  toggleDemo,
  trace,
} from "../store";
import { DEMO_LABEL } from "../store";
import { durationText } from "../model";
import { Badge, Button, Card, Divider, Dot, Kv, Small, Strong, p, SMALL, TINY } from "../components/ui";

function traceBody() {
  const loaded = trace();
  if (loaded === null) {
    return <Small text="点一个告警或 Q-time 违规，或在左侧输入 event_id，即可看到这条链。" />;
  }
  const tables = Object.entries(loaded.greptime)
    .map(([table, rows]) => `${table} ${rows.length} 行`)
    .join(" · ");
  const outbox = loaded.outbox;
  return (
    <View style={{ gap: 4 }}>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Text style={{ fontSize: SMALL, fontWeight: "600", color: p().text }}>
          {`${loaded.event.event.kind} · ${loaded.event.event.lot_id}`}
        </Text>
        <View style={{ flex: 1 }} />
        <Button label="复制 event_id" onPress={copyTraceEvent} />
      </View>
      <Kv key="event_id" value={loaded.event_id} />
      <Kv
        key="outbox"
        value={
          outbox === null
            ? "无 outbox 行"
            : outbox.published_at === null
              ? `待投递 · 尝试 ${outbox.attempts} 次`
              : `已投递 ${outbox.published_at}`
        }
      />
      <Kv key="GreptimeDB" value={tables === "" ? "无派生行（该事件不产指标）" : tables} />
      {outbox?.last_error === null || outbox?.last_error === undefined ? null : (
        <Small text={`最后一次投递错误：${outbox.last_error}`} />
      )}
      <Small text="追这条链的口径：PG 为事实来源，outbox 至少一次投递，Greptime 行以 event_id 幂等。" />
    </View>
  );
}

function monitorBody() {
  const current = monitor();
  if (current === null) return <Small text="读取 monitor…" />;
  return (
    <View style={{ gap: 4 }}>
      {current.components.map((component) => (
        <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
          <Dot
            color={
              component.status === "up" ? p().good : component.status === "degraded" ? p().warn : p().bad
            }
          />
          <Text style={{ fontSize: SMALL, color: p().text }}>{component.name}</Text>
          <View style={{ flex: 1 }} />
          <Small text={`${component.detail} · ${component.latency_ms} ms`} />
        </View>
      ))}
      <View style={{ flexDirection: "row" }}>
        <Button label="重新巡查" onPress={() => void loadMonitor()} />
      </View>
    </View>
  );
}

function chaosBody() {
  const samples = chaos();
  const pending = outboxPending();
  if (samples.length === 0) {
    return (
      <Small
        text={
          pending === 0
            ? "无积压：outbox 已追平。运行 ./scripts/chaos.sh 时这里会画出积压曲线"
            : "有积压但尚无采样：投递者正在追平"
        }
      />
    );
  }
  const peak = Math.max(peakOf(samples), 1);
  let start: number | null = null;
  let drained: number | null = null;
  for (const sample of samples) {
    if (sample.pending > 0) {
      start = sample.at_ms;
      drained = null;
    } else if (start !== null && drained === null) {
      drained = sample.at_ms;
    }
  }
  const drainSeconds = start !== null && drained !== null ? (drained - start) / 1000 : null;
  return (
    <View style={{ gap: 4 }}>
      <View
        style={{
          height: 46,
          flexDirection: "row",
          alignItems: "flex-end",
          gap: 2,
          padding: 4,
          borderRadius: 7,
          backgroundColor: p().surfaceAlt,
        }}
      >
        {samples.map((sample) => (
          <View
            style={{
              flex: 1,
              minWidth: 2,
              height: Math.max(1, (sample.pending / peak) * 34),
              backgroundColor: sample.pending > 0 ? p().warn : p().good,
            }}
          />
        ))}
      </View>
      <Small
        text={
          drainSeconds === null
            ? `峰值积压 ${peak} · 尚在追平`
            : `峰值积压 ${peak} · 追平 ${drainSeconds.toFixed(1)}s · 零丢失（至少一次 + event_id 幂等）`
        }
      />
    </View>
  );
}

function peakOf(samples: { pending: number }[]): number {
  return samples.reduce((max, sample) => Math.max(max, sample.pending), 0);
}

function kpiBody() {
  const current = analysis();
  if (current === null) return <Small text="读取 analytics…" />;
  const kpi = current.kpi;
  const downtime = kpi.tools.reduce((total, tool) => total + tool.down_ms, 0);
  return (
    <View style={{ gap: 4 }}>
      <Kv
        key="平均周期"
        value={kpi.mean_cycle_time_ms === null ? "—" : durationText(Math.round(kpi.mean_cycle_time_ms))}
      />
      <Kv key="产出 / 小时" value={kpi.throughput_lots_per_hour === null ? "—" : kpi.throughput_lots_per_hour.toFixed(2)} />
      <Kv key="FPY" value={kpi.first_pass_yield === null ? "—" : `${(kpi.first_pass_yield * 100).toFixed(1)}%`} />
      <Kv key="WIP" value={`${kpi.wip_lots} 批`} />
      <Kv key="停机合计" value={durationText(downtime)} />
      <Divider />
      <View style={{ gap: 2 }}>
        {current.notes.map((note) => (
          <View style={{ flexDirection: "row", gap: 8 }}>
            <Text style={{ fontSize: TINY, color: p().accent }}>{note.metric}</Text>
            <Small text={`${note.source} · ${note.formula}`} />
          </View>
        ))}
      </View>
    </View>
  );
}

function drillsBody() {
  const started = backup();
  const drilled = restore();
  return (
    <View style={{ gap: 4 }}>
      <Text style={{ fontSize: SMALL, color: p().muted }}>
        {started === null
          ? "尚无备份：运行 ./scripts/backup.sh"
          : `备份 ${started.taken_at ?? ""} · base ${started.pg_full_backup ?? ""} · WAL ${
              started.wal_continuity ?? ""
            } · Greptime ${started.greptime_snapshot ?? ""}`}
      </Text>
      <Text style={{ fontSize: SMALL, color: p().muted }}>
        {drilled === null
          ? "尚无恢复演练：运行 ./scripts/restore.sh"
          : `PITR ${drilled.target_time ?? ""} · lots ${drilled.lots ?? ""} · events ${
              drilled.events ?? ""
            } · Greptime ${drilled.greptime_rows ?? ""} 行 · RTO ${((drilled.elapsed_ms ?? 0) / 1000).toFixed(
              1,
            )}s · RPO 0（WAL 连续）`}
      </Text>
      <Small text="RTO 含容器/进程重启；RPO 由 WAL 连续性与演练校验给出，不在客户端计算。" />
    </View>
  );
}

function demoBody() {
  return (
    <View style={{ gap: 4 }}>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Small text={DEMO_LABEL[demo()] ?? "演示待启动"} />
        <View style={{ flex: 1 }} />
        <Button label={demoRunning() ? "停止演示" : "开始演示"} accent onPress={toggleDemo} />
      </View>
      <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 4 }}>
        <Button label="导出场景快照" onPress={exportScene} />
        <Button label="导出批次追溯" onPress={exportLot} />
        <Button label="导出 chaos 报告" onPress={() => void exportChaos()} />
        <Button label="启动本地后端（演示触发）" onPress={() => void startBackend()} />
      </View>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Button
          label={reduceMotion() ? "减少动效 ✓" : "减少动效"}
          onPress={() => reduceMotion.set(!reduceMotion())}
        />
        <Button label={dense() ? "标签密度：标准" : "标签密度：精简"} onPress={() => dense.set(!dense())} />
        <Small text={`后端：${backend()}`} />
      </View>
      <Button label="重放派生数据（演示触发）" onPress={() => sim("/admin/replay", "Replay 派生数据")} />
    </View>
  );
}

function secsBody() {
  const current = secs();
  if (current === null) return <Small text="SECS/GEM 已关闭" />;
  return (
    <View style={{ gap: 4 }}>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Dot color={current.host_connected ? p().good : p().bad} />
        <Small text={current.host_connected ? "链路已建立" : "链路断开"} />
        <View style={{ flex: 1 }} />
        <Button label="断链 1.5s" onPress={() => sim("/api/secs/drop", "SECS 断链")} />
      </View>
      <Text style={{ fontSize: SMALL, color: p().muted }}>
        {`S1F1 ${current.s1f1_ok} · S6F11 ${current.s6f11_sent} · S5F1 ${current.s5f1_sent} · S7F3 ${current.s7f3_uploaded} · S7F6 ${current.s7f6_verified}`}
      </Text>
      <Text style={{ fontSize: SMALL, color: p().muted }}>
        {`补传缓冲 ${current.spool_depth} · 已补传 ${current.spooled_replayed} · 断链 ${current.link_drops} 次`}
      </Text>
    </View>
  );
}

export function CrossPanel() {
  return (
    <View style={{ gap: 12 }}>
      <Card title="L3 事件 → outbox → 数据点" source="lot_event.event_id / outbox / GreptimeDB">
        {traceBody()}
      </Card>
      <Card title="组件巡查 · SmartFactory Monitor" source="PG / GreptimeDB / 对象存储 / oxide-api / outbox 投递者">
        {monitorBody()}
      </Card>
      <Card title="消息总线 · outbox 积压与追平" source="outbox WHERE published_at IS NULL → published_event">
        <View style={{ gap: 8 }}>
          <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
            <View style={{ gap: 2 }}>
              <Text
                style={{
                  fontSize: 24,
                  fontWeight: "700",
                  color: outboxPending() > 0 ? p().warn : p().good,
                }}
              >
                {outboxPending()}
              </Text>
              <Small text="待投递" />
            </View>
            <View style={{ gap: 2 }}>
              <Text style={{ fontSize: 18, fontWeight: "600", color: p().text }}>{outboxPublished()}</Text>
              <Small text="已投递" />
            </View>
            <View style={{ flex: 1 }} />
            <Small
              text={
                greptimeOk()
                  ? "GreptimeDB 在线：追平中会按序下降"
                  : "GreptimeDB 断流：事件只积压在 PG，恢复后按序追平"
              }
            />
          </View>
          {chaosBody()}
        </View>
      </Card>
      <Card title="KPI · 口径化指标" source="lot_event 派生 · 每个数字标注公式">
        {kpiBody()}
      </Card>
      <Card title="备份与恢复演练 · RPO / RTO" source="backup.sh / restore.sh / chaos.sh 产物">
        {drillsBody()}
      </Card>
      <Card title="演示、导出与可配置项" source="写操作均标注 演示触发">
        {demoBody()}
      </Card>
      <Card title="SECS/GEM 假机台" source="HSMS-SS · E4/E5/E30/E37">
        {secsBody()}
      </Card>
    </View>
  );
}
