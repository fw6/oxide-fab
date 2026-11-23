/** MES: WIP by step, the tool three-state view, and lot genealogy. */
import { Text, View } from "nsh";
import {
  analysis,
  detail,
  drill,
  hold,
  lots,
  mergeLot,
  release,
  rework,
  scrapWafer,
  selectedLot,
  selectedWafer,
  splitLot,
  tools,
  steps,
} from "../store";
import {
  defectFill,
  durationText,
  lightColor,
  stateColor,
  type LotView,
  type StepView,
  type ToolView,
} from "../model";
import { Badge, Bar, Button, Card, Divider, Dot, Small, Strong, p, BODY, SMALL, TINY } from "../components/ui";

/** The tools standing at a step now, read live so a fold shows at once. */
function busyLine(step: StepView) {
  const ids = tools()
    .filter((tool) => tool.current_step === step.step_no)
    .map((tool) => tool.id);
  return <Text style={{ fontSize: TINY, color: p().muted }}>{ids.length === 0 ? "本工序无在机批次" : `在机 ${ids.join(" · ")}`}</Text>;
}

function StepBlock(props: { step: StepView }) {
  const step = props.step;
  return (
    <View style={{ gap: 4, padding: 8, borderRadius: 8, backgroundColor: p().surfaceAlt }}>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Text style={{ fontSize: BODY, fontWeight: "600", color: p().text }}>
          {`S${step.step_no} · ${step.name}`}
        </Text>
        <View style={{ flex: 1 }} />
        <Text style={{ fontSize: TINY, color: p().muted }}>
          {`排队 ${step.queue_len} · 机台 ${step.busy_tools}/${step.total_tools}`}
        </Text>
      </View>
      <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 4 }}>
        {step.current_lots.slice(0, 6).map((id) => (
          <View
            onPress={() => drill({ kind: "foup", lot_id: id })}
            style={{
              paddingHorizontal: 6,
              paddingVertical: 2,
              borderRadius: 5,
              backgroundColor: p().surface,
              borderWidth: 1,
              borderColor: p().border,
            }}
          >
            <Text style={{ fontSize: TINY, color: p().info }}>{id}</Text>
          </View>
        ))}
      </View>
      {busyLine(step)}
    </View>
  );
}

/** The E116 utilisation line, read from analytics in a binding. */
function e116Line(tool: ToolView) {
  const kpi = analysis()?.kpi.tools.find((candidate) => candidate.equipment_id === tool.id);
  if (kpi === undefined) return <Text style={{ fontSize: SMALL, color: p().muted }}>等待 e116</Text>;
  return (
    <View style={{ flexDirection: "row", alignItems: "center", gap: 4 }}>
      <Bar ratio={kpi.utilization} color={p().good} width={70} />
      <Text style={{ fontSize: TINY, color: p().muted }}>
        {`效率 ${Math.round(kpi.utilization * 100)}% · ${durationText(kpi.busy_ms)} · 停 ${durationText(kpi.down_ms)} · ${kpi.track_outs} 次`}
      </Text>
    </View>
  );
}

function ToolRow(props: { tool: ToolView }) {
  const tool = props.tool;
  return (
    <View
      onPress={() => drill({ kind: "tool", equipment_id: tool.id })}
      style={{
        flexDirection: "row",
        alignItems: "center",
        gap: 8,
        paddingHorizontal: 8,
        paddingVertical: 6,
        borderRadius: 7,
        backgroundColor: p().surfaceAlt,
      }}
    >
      <Dot color={lightColor(p(), tool.light)} />
      <Text style={{ fontSize: BODY, fontWeight: "600", color: p().text }}>{tool.id}</Text>
      <Text style={{ fontSize: TINY, color: p().muted }}>
        {tool.current_lot === null ? tool.state : `${tool.state} · ${tool.current_lot}`}
      </Text>
      <View style={{ flex: 1 }} />
      {e116Line(tool)}
    </View>
  );
}

/** The selected wafer's genealogy, read from the loaded detail. */
function genealogyBody() {
  const waferId = selectedWafer();
  const entry = detail()?.genealogy.find((candidate) => candidate.wafer_id === waferId);
  if (entry === undefined) return <Text style={{ fontSize: SMALL, color: p().muted }}>读取逐片履历…</Text>;
  const last = entry.steps.at(-1);
  return (
    <View style={{ gap: 4, padding: 8, borderRadius: 7, backgroundColor: p().surfaceAlt }}>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Strong text={entry.wafer_id} />
        <Small
          text={`${entry.state} · 返工 ${entry.rework_count} 次${
            entry.defect_info === null ? "" : ` · 缺陷 ${entry.defect_info}`
          }`}
        />
        <View style={{ flex: 1 }} />
        {last === undefined ? (
          <View />
        ) : (
          <Button label="来源机台" onPress={() => drill({ kind: "tool", equipment_id: last.equipment_id })} />
        )}
      </View>
      {entry.steps.map((step) => (
        <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
          <Text style={{ fontSize: TINY, color: p().muted }}>{`S${step.step_no}`}</Text>
          <Text style={{ fontSize: TINY, color: p().info }}>{step.equipment_id}</Text>
          <Text style={{ fontSize: TINY, color: p().muted }}>{step.recipe_id}</Text>
          <View style={{ flex: 1 }} />
          <Text style={{ fontSize: TINY, color: p().text }}>
            {step.track_out_ms > 0 ? durationText(step.duration_ms) : "进行中"}
          </Text>
          <Text style={{ fontSize: TINY, color: p().warn }}>
            {step.measured === null ? "" : `${step.measured.toFixed(2)} nm`}
          </Text>
        </View>
      ))}
    </View>
  );
}

function WaferGrid(props: { lot: LotView }) {
  const lot = props.lot;
  return (
    <View style={{ gap: 8, paddingTop: 4 }}>
      <Divider />
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Text style={{ fontSize: SMALL, color: p().muted }}>{`${lot.id} · 批次操作`}</Text>
        <View style={{ flex: 1 }} />
        <Badge label="演示触发" fg={p().warn} />
      </View>
      <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 4 }}>
        <Button label="Hold" onPress={hold} />
        <Button label="Release" onPress={release} />
        <Button label="返工" onPress={rework} />
        <Button label="拆分" onPress={splitLot} />
        <Button label="合并" onPress={mergeLot} />
        <Button label="报废首片" onPress={scrapWafer} />
      </View>
      <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 4 }}>
        {Array.from({ length: 25 }, (_, index) => index + 1).map((slot) => {
          const wafer = lot.wafers.find((candidate) => candidate.slot === slot);
          const scrapped = wafer !== undefined && wafer.state === "scrapped";
          const fill =
            wafer === undefined
              ? p().background
              : scrapped
                ? (defectFill(p(), wafer.defect_info) ?? p().bad)
                : wafer.state === "done"
                  ? p().good
                  : wafer.state === "processing"
                    ? p().info
                    : p().surfaceAlt;
          const border =
            wafer === undefined
              ? p().border
              : scrapped
                ? p().bad
                : wafer.state === "done"
                  ? p().good
                  : wafer.state === "processing"
                    ? p().info
                    : p().border;
          return (
            <View
              onPress={() => {
                if (wafer !== undefined) drill({ kind: "wafer", lot_id: lot.id, wafer_id: wafer.id });
              }}
              style={{
                width: 26,
                height: 26,
                alignItems: "center",
                justifyContent: "center",
                borderRadius: 5,
                borderWidth: 1,
                borderColor: border,
                backgroundColor: fill,
                opacity: selectedWafer() === wafer?.id ? 1 : 0.92,
              }}
            >
              <Text style={{ fontSize: 8.5, color: p().text }}>
                {wafer === undefined ? String(slot).padStart(2, "0") : `W${String(slot).padStart(2, "0")}`}
              </Text>
            </View>
          );
        })}
      </View>
      {selectedWafer() !== null ? (
        genealogyBody()
      ) : (
        <Text style={{ fontSize: SMALL, color: p().muted }}>点一个槽位看逐片履历；红色为缺陷分类</Text>
      )}
    </View>
  );
}

function LotRow(props: { lot: LotView }) {
  const lot = props.lot;
  return (
    <View
      onPress={() => drill({ kind: "foup", lot_id: lot.id })}
      style={{
        gap: 4,
        padding: 8,
        borderRadius: 8,
        borderWidth: 1,
        backgroundColor: selectedLot() === lot.id ? p().surfaceAlt : p().surface,
        borderColor: selectedLot() === lot.id ? p().accent : p().border,
      }}
    >
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Dot color={stateColor(p(), lot.state)} />
        <Text style={{ fontSize: BODY, fontWeight: "600", color: p().text }}>{lot.id}</Text>
        <Text style={{ fontSize: TINY, color: stateColor(p(), lot.state) }}>{lot.state}</Text>
        <View style={{ flex: 1 }} />
        <Text style={{ fontSize: TINY, color: p().muted }}>
          {`S${lot.current_step} · ${lot.done_wafers}/${lot.wafer_count} 片 · P${lot.priority}`}
        </Text>
      </View>
      {selectedLot() === lot.id && detail() !== null ? <WaferGrid lot={lot} /> : null}
    </View>
  );
}

export function MesPanel() {
  return (
    <View style={{ gap: 12 }}>
      <Card title="工序队列 · WIP" source="route_step + lot · 队列只影响显示，不排序仿真">
        <View style={{ gap: 6 }}>
          {steps().map((step) => (
            <StepBlock step={step} />
          ))}
        </View>
      </Card>
      <Card title="机台三态 · E116 时长" source="equipment.state / e116_perf 聚合 · 绿运行 黄维护 红停机">
        <View style={{ gap: 4 }}>
          {tools().map((tool) => (
            <ToolRow tool={tool} />
          ))}
        </View>
      </Card>
      <Card title="批次与逐片追溯" source="lot / wafer / lot_event · 点击批次进入 L2">
        <View style={{ gap: 4 }}>
          {lots().map((lot) => (
            <LotRow lot={lot} />
          ))}
        </View>
      </Card>
    </View>
  );
}
