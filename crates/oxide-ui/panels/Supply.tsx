/** Supply: planned vs actual step cycles, and delivery drift. */
import { Text, View } from "nsh";
import { analysis } from "../store";
import { durationText } from "../model";
import { Badge, Bar, Card, Dot, Kv, Small, p, SMALL, TINY } from "../components/ui";

function planBody() {
  const current = analysis();
  if (current === null) return <Small text="读取 analytics…" />;
  const cycles = current.kpi.step_cycles;
  if (cycles.length === 0) return <Small text="等待工序样本" />;
  return (
    <View style={{ gap: 8 }}>
      {cycles.map((cycle) => {
        const planned = cycle.planned_ms;
        const actual = cycle.actual_mean_ms ?? 0;
        const ratio = planned > 0 ? actual / planned : 0;
        const late = ratio > 1.15 && cycle.samples > 0;
        const color = late ? p().warn : p().good;
        return (
          <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
            <Dot color={color} />
            <Text style={{ fontSize: SMALL, color: p().muted }}>{`S${cycle.step_no}`}</Text>
            <Small
              text={`计划 ${durationText(planned)} → 实际 ${
                cycle.samples > 0 ? durationText(Math.round(actual)) : "—"
              }（${cycle.samples} 次）`}
            />
            <View style={{ flex: 1 }} />
            <Bar ratio={ratio / 2} color={color} width={70} />
            <Text style={{ fontSize: TINY, color: p().text }}>{`${Math.round(ratio * 100)}%`}</Text>
          </View>
        );
      })}
    </View>
  );
}

function deliveryBody() {
  const current = analysis();
  if (current === null) return <Small text="读取 analytics…" />;
  const cycles = current.kpi.step_cycles;
  const planned = cycles.reduce((total, cycle) => total + cycle.planned_ms, 0);
  const lots = current.kpi.lot_cycles;
  if (lots.length === 0) {
    return <Small text={`尚无完成批次；计划周期 ${durationText(planned)}`} />;
  }
  const worst = lots.reduce((ratio, lot) => Math.max(ratio, lot.cycle_ms / Math.max(planned, 1)), 0);
  return (
    <View style={{ gap: 4 }}>
      <Kv key="计划周期" value={durationText(planned)} />
      {worst <= 1.15 ? (
        <Small text="无交付偏差预警：实际周期在计划 +15% 以内" />
      ) : (
        <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
          <Badge label={`偏差 ${Math.round((worst - 1) * 100)}%`} fg={p().bad} />
          <Small text="存在超计划批次，见下方逐批周期" />
        </View>
      )}
      {lots.slice(0, 6).map((lot) => {
        const ratio = lot.cycle_ms / Math.max(planned, 1);
        return (
          <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
            <Text style={{ fontSize: SMALL, color: p().text }}>{lot.lot_id}</Text>
            <View style={{ flex: 1 }} />
            <Small text={durationText(lot.cycle_ms)} />
            <Text style={{ fontSize: TINY, color: ratio > 1.15 ? p().warn : p().good }}>
              {`${Math.round(ratio * 100)}%`}
            </Text>
          </View>
        );
      })}
    </View>
  );
}

export function SupplyPanel() {
  return (
    <View style={{ gap: 12 }}>
      <Card title="计划节拍 vs 实际节拍" source="route_step.duration_ms ↔ track_out.duration_ms">
        {planBody()}
      </Card>
      <Card title="交付偏差 · 早期预警" source="批次周期 = 首事件 → lot_completed；计划 = 四道工序之和">
        {deliveryBody()}
      </Card>
    </View>
  );
}
