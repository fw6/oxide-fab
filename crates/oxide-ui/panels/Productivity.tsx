/** Productivity: offline what-if, dispatch, WIP bubbles, utilisation, auto/step. */
import { Text, View } from "nsh";
import {
  analysis,
  dispatch,
  drill,
  runWhatIf,
  running,
  sim,
  steps,
  whatif,
  whatifAddTool,
  whatifBusy,
  whatifFifo,
} from "../store";
import { durationText } from "../model";
import { Badge, Bar, Button, Card, Dot, Small, p, BODY, SMALL, TINY } from "../components/ui";

function delta(value: number, digits: number): string {
  const sign = value >= 0 ? "+" : "";
  return `${sign}${value.toFixed(digits)}`;
}

function whatIfBody() {
  const result = whatif();
  if (result === null) return <View />;
  const row = (label: string, baseline: string, variant: string, change: string) => (
    <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
      <Text style={{ fontSize: SMALL, color: p().muted }}>{label}</Text>
      <View style={{ flex: 1 }} />
      <Text style={{ fontSize: SMALL, color: p().text }}>{`${baseline} → ${variant}`}</Text>
      <Text style={{ fontSize: SMALL, fontWeight: "600", color: p().accent }}>{change}</Text>
    </View>
  );
  return (
    <View style={{ gap: 4 }}>
      {row(
        "完成批次",
        result.baseline.completed_lots.toFixed(1),
        result.variant.completed_lots.toFixed(1),
        delta(result.delta.completed_lots, 0),
      )}
      {row(
        "平均周期",
        (result.baseline.mean_cycle_time_ms ?? 0).toFixed(1),
        (result.variant.mean_cycle_time_ms ?? 0).toFixed(1),
        delta(result.delta.mean_cycle_time_ms ?? 0, 0),
      )}
      {row(
        "产出/小时",
        (result.baseline.throughput_lots_per_hour ?? 0).toFixed(1),
        (result.variant.throughput_lots_per_hour ?? 0).toFixed(1),
        delta(result.delta.throughput_lots_per_hour ?? 0, 2),
      )}
      {row("WIP", String(result.baseline.wip_end), String(result.variant.wip_end), delta(result.delta.wip_end, 0))}
      <View style={{ flexDirection: "row", gap: 4 }}>
        <Badge label={result.variant.added_tool ?? "未加机台"} fg={p().accent} />
        {result.baseline.from_start ? <Badge label="从 t0 重放" fg={p().warn} /> : null}
      </View>
      <Small text={result.notes.join(" ")} />
    </View>
  );
}

function dispatchBody() {
  const decisions = dispatch();
  if (decisions.length === 0) {
    return <Small text="尚无派工：运行仿真后，每次上料都会带着规则与队列位置出现在这里" />;
  }
  return (
    <View style={{ gap: 4 }}>
      {decisions.slice(0, 7).map((decision) => (
        <View
          onPress={() => drill({ kind: "foup", lot_id: decision.lot_id })}
          style={{ gap: 4, paddingHorizontal: 6, paddingVertical: 4, borderRadius: 5, backgroundColor: p().surfaceAlt }}
        >
          <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
            <Text style={{ fontSize: SMALL, fontWeight: "600", color: p().text }}>
              {`${decision.lot_id} → ${decision.equipment_id}`}
            </Text>
            <View style={{ flex: 1 }} />
            <Badge label={decision.rule} fg={p().accent} bg={p().surface} />
          </View>
          <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
            <Small
              text={`S${decision.step_no} · 优先级 ${decision.priority} · 队列 ${decision.queue_position + 1}/${decision.queue_len}`}
            />
            <View style={{ flex: 1 }} />
            <Small text={`资格 ${decision.qualified_tools.length} 台 · ${decision.recipe_id}`} />
          </View>
        </View>
      ))}
    </View>
  );
}

function wipBody() {
  const current = steps();
  if (current.length === 0) return <Small text="等待快照" />;
  const peak = current.reduce((max, step) => Math.max(max, step.queue_len), 0);
  const bottleneck = current.filter((step) => step.queue_len === peak);
  return (
    <View style={{ gap: 8 }}>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
        {current.map((step) => {
          const diameter = 28 + Math.min(step.queue_len * 3, 28);
          return (
            <View style={{ alignItems: "center", gap: 4 }}>
              <View
                style={{
                  width: diameter,
                  height: diameter,
                  borderRadius: diameter / 2,
                  alignItems: "center",
                  justifyContent: "center",
                  backgroundColor: step.queue_len === peak && peak > 0 ? p().warn : p().info,
                }}
              >
                <Text style={{ fontSize: SMALL, fontWeight: "700", color: p().accentText }}>{step.queue_len}</Text>
              </View>
              <Small text={`S${step.step_no}`} />
            </View>
          );
        })}
      </View>
      <Small
        text={
          peak === 0
            ? "无排队：瓶颈不适用"
            : `当前瓶颈：${bottleneck.map((step) => `S${step.step_no} ${step.name}`).join(" · ")}`
        }
      />
    </View>
  );
}

function utilisationBody() {
  const current = analysis();
  if (current === null) return <Small text="读取 analytics…" />;
  const windowMs =
    current.kpi.window_start_ms !== null && current.kpi.window_end_ms !== null
      ? current.kpi.window_end_ms - current.kpi.window_start_ms
      : 0;
  return (
    <View style={{ gap: 4 }}>
      {current.kpi.tools.map((tool) => {
        const idle = Math.max(0, windowMs - tool.busy_ms - tool.down_ms);
        return (
          <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
            <Dot color={p().info} />
            <Text style={{ fontSize: SMALL, color: p().text }}>{tool.equipment_id}</Text>
            <View style={{ flex: 1 }} />
            <Text style={{ fontSize: TINY, color: p().muted }}>
              {`运行 ${Math.round(tool.utilization * 100)}% · 停机 ${Math.round(
                (tool.down_ms / Math.max(windowMs, 1)) * 100,
              )}% · 空窗 ${durationText(idle)}`}
            </Text>
            <Bar ratio={idle / Math.max(windowMs, 1)} color={p().muted} width={54} />
          </View>
        );
      })}
    </View>
  );
}

function autoBody() {
  const current = analysis();
  const windowMs =
    current !== null && current.kpi.window_start_ms !== null && current.kpi.window_end_ms !== null
      ? current.kpi.window_end_ms - current.kpi.window_start_ms
      : 0;
  const idle =
    current === null
      ? 0
      : current.kpi.tools.reduce(
          (total, tool) => total + Math.max(0, windowMs - tool.busy_ms - tool.down_ms),
          0,
        );
  return (
    <View style={{ gap: 4 }}>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Dot color={running() ? p().good : p().warn} />
        <Small text={running() ? "事件驱动自动流转中" : "已暂停：单步推进，用于对比空窗"} />
        <View style={{ flex: 1 }} />
        <Button label="自动" onPress={() => sim("/api/sim/play", "自动流转")} />
        <Button label="单步" onPress={() => sim("/api/sim/step", "单步")} />
      </View>
      {current === null ? null : (
        <Small text={`当前口径：全部机台空窗合计 ${durationText(idle)}（窗口 ${durationText(windowMs)}）`} />
      )}
    </View>
  );
}

export function ProductivityPanel() {
  return (
    <View style={{ gap: 12 }}>
      <Card title="离线 what-if 重放" source="snapshot 副本重放 · 不打断当前实例">
        <View style={{ gap: 8 }}>
          <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
            <Button
              label={() => (whatifFifo() ? "规则 FIFO ✓" : "规则 优先级→FIFO")}
              onPress={() => whatifFifo.set(!whatifFifo())}
            />
            <Button
              label={() => (whatifAddTool() ? "加一台同型机 ✓" : "加一台同型机")}
              onPress={() => whatifAddTool.set(!whatifAddTool())}
            />
            <View style={{ flex: 1 }} />
            <Button
              label={() => (whatifBusy() ? "重放中…" : "运行重放")}
              accent
              onPress={() => void runWhatIf()}
            />
          </View>
          {whatIfBody()}
        </View>
      </Card>
      <Card title="派工决策 · What-Next / Where-Next" source="oxide-sim 规则 + route_step 资格矩阵">
        {dispatchBody()}
      </Card>
      <Card title="WIP 气泡与瓶颈" source="route_step 队列长度 · 口径：当前排队批次数">
        {wipBody()}
      </Card>
      <Card title="设备利用率 · 空窗" source="track_out.duration_ms / 事件窗口">
        {utilisationBody()}
      </Card>
      <Card title="自动 / 单步" source="FullAuto 与手动单步共用同一组接口">
        {autoBody()}
      </Card>
    </View>
  );
}
