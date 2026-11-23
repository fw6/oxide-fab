/** The window: header, left rail, viewport column, inspector and footer. */
import { Scroll, Text, View } from "nsh";
import { Input } from "nsh/base";
import {
  TAB_ORDER,
  adapter,
  connected,
  dark,
  demo,
  demoRunning,
  DEMO_LABEL,
  drill,
  eventsTotal,
  explosion,
  gotoLevel,
  greptimeOk,
  lastAction,
  levelLabel,
  levelUp,
  loadTrace,
  lots,
  note,
  nowMs,
  outboxPending,
  publishedEvents,
  renderError,
  running,
  search,
  select,
  selected,
  sim,
  speed,
  startDemo,
  stopDemo,
  tab,
  tabHint,
  tabLabel,
  toggleDemo,
  toggleProjection,
  tools,
  traceEvent,
  viewport,
} from "../store";
import { clockText, entityLabel, lotIdOf, toolIdOf, type Entity, type TabId } from "../model";
import { ViewportPane } from "./Viewport";
import { Button, Dot, Small, Strong, p, SMALL, TINY } from "./ui";
import { MesPanel } from "../panels/Mes";
import { QualityPanel } from "../panels/Quality";
import { SupplyPanel } from "../panels/Supply";
import { ProductivityPanel } from "../panels/Productivity";
import { CrossPanel } from "../panels/Cross";

const RAIL_WIDTH = 244;
const INSPECTOR_WIDTH = 430;

function TabButton(props: { id: TabId }) {
  return (
    <View
      onPress={() => {
        stopDemo();
        tab.set(props.id);
      }}
      style={{
        paddingHorizontal: 8,
        paddingVertical: 4,
        borderRadius: 6,
        backgroundColor: tab() === props.id ? p().surfaceAlt : p().surface,
        borderBottomWidth: 1,
        borderColor: tab() === props.id ? p().accent : p().surface,
      }}
    >
      <Text
        style={{
          fontSize: SMALL,
          fontWeight: tab() === props.id ? "600" : "400",
          color: tab() === props.id ? p().text : p().muted,
        }}
      >
        {tabLabel(props.id)}
      </Text>
    </View>
  );
}

function SimControls() {
  return (
    <View style={{ flexDirection: "row", alignItems: "center", gap: 4 }}>
      <Button
        label={() => (running() ? "暂停" : "运行")}
        onPress={() => sim(running() ? "/api/sim/pause" : "/api/sim/play", running() ? "暂停" : "运行")}
      />
      <Button label="单步" onPress={() => sim("/api/sim/step", "单步")} />
      <Button
        label="−"
        onPress={() => {
          const next = Math.max(speed() / 2, 0.5);
          sim(`/api/sim/speed/${next}`, "降速");
        }}
      />
      <Text style={{ fontSize: TINY, color: p().muted }}>{`×${speed().toFixed(0)}`}</Text>
      <Button
        label="+"
        onPress={() => {
          const next = Math.min(speed() * 2, 100);
          sim(`/api/sim/speed/${next}`, "提速");
        }}
      />
    </View>
  );
}

function Header() {
  return (
    <View
      style={{
        width: "100%",
        paddingHorizontal: 12,
        paddingVertical: 8,
        flexDirection: "row",
        alignItems: "center",
        flexWrap: "wrap",
        gap: 12,
        backgroundColor: p().surface,
        borderBottomWidth: 1,
        borderColor: p().border,
      }}
    >
      <View style={{ gap: 2 }}>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
          <Text style={{ fontSize: 17, fontWeight: "700", color: p().text }}>氧化层 · oxide-fab</Text>
          <Text style={{ fontSize: TINY, color: p().accent }}>3D 工作台</Text>
        </View>
        <Text style={{ fontSize: TINY, color: p().muted }}>仿真产线 · PG 事实来源 · 事件驱动</Text>
      </View>
      <View style={{ flex: 1 }} />
      <View style={{ flexDirection: "row", alignItems: "center", gap: 4 }}>
        {TAB_ORDER.map((id) => (
          <TabButton id={id} />
        ))}
      </View>
      <View style={{ flex: 1 }} />
      <View style={{ flexDirection: "row", alignItems: "center", gap: 6 }}>
        <Dot color={() => (connected() ? p().good : p().bad)} />
        <Text style={{ fontSize: TINY, color: p().muted }}>{connected() ? "实时" : "重连中"}</Text>
        <Text style={{ fontSize: TINY, color: p().muted }}>{clockText(nowMs())}</Text>
      </View>
      <SimControls />
      <View style={{ flexDirection: "row", alignItems: "center", gap: 4 }}>
        <Button label={() => (dark() ? "浅色" : "深色")} onPress={() => dark.set(!dark())} />
        <Button label={() => (demoRunning() ? "停止演示" : "演示")} accent onPress={toggleDemo} />
      </View>
    </View>
  );
}

function selectionDetail(entity: Entity) {
  const tool = toolIdOf(entity);
  if (tool !== null) {
    const found = tools().find((candidate) => candidate.id === tool);
    if (found === undefined) return "机台不在快照中";
    return `${found.kind} · ${found.state} · 在机 ${found.current_lot ?? "—"}`;
  }
  const lot = lotIdOf(entity);
  if (lot !== null) {
    const found = lots().find((candidate) => candidate.id === lot);
    if (found === undefined) return "批次不在快照中";
    return `${found.state} · S${found.current_step} · ${found.done_wafers}/${found.wafer_count} 片`;
  }
  return "";
}

function BreadcrumbCard() {
  return (
    <View
      style={{
        width: "100%",
        padding: 8,
        gap: 4,
        borderRadius: 10,
        backgroundColor: p().surface,
        borderWidth: 1,
        borderColor: p().border,
      }}
    >
      <Text style={{ fontSize: SMALL, fontWeight: "600", color: p().text }}>{levelLabel()}</Text>
      <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 4 }}>
        <Button label="L0" onPress={() => gotoLevel(0)} />
        <Button label="L1" onPress={() => gotoLevel(1)} />
        <Button label="L2" onPress={() => gotoLevel(2)} />
        <Button label="L3" onPress={() => gotoLevel(3)} />
      </View>
      <View style={{ flexDirection: "row", gap: 4 }}>
        <Button label="← 退回" onPress={levelUp} />
        <Button label="复位视角" onPress={() => viewport.resetCamera()} />
      </View>
    </View>
  );
}

function SelectionCard() {
  return (
    <View
      style={{
        width: "100%",
        padding: 12,
        gap: 8,
        borderRadius: 10,
        backgroundColor: p().surface,
        borderWidth: 1,
        borderColor: p().border,
      }}
    >
      <Text style={{ fontSize: 13.5, fontWeight: "600", color: p().text }}>当前选中</Text>
      {selected() === null ? (
        <Small text="点场景或点列表：机台 → 部件，批次 → 晶圆，事件 → 数据点" />
      ) : (
        <View style={{ gap: 4 }}>
          <Strong text={() => (selected() === null ? "" : entityLabel(selected() as Entity))} />
          {selected() === null ? null : <Small text={() => selectionDetail(selected() as Entity)} />}
          <View style={{ flexDirection: "row", gap: 4 }}>
            <Button
              label="打开详情"
              onPress={() => {
                const entity = selected();
                if (entity === null) return;
                if (entity.kind === "tool" || entity.kind === "toolPart") {
                  drill({ kind: "tool", equipment_id: entity.equipment_id });
                } else if (entity.kind === "foup" || entity.kind === "wafer") {
                  drill({ kind: "foup", lot_id: entity.lot_id });
                }
              }}
            />
            <Button label="清空" onPress={() => select(null)} />
          </View>
        </View>
      )}
    </View>
  );
}

function searchHits(): Entity[] {
  const query = search().trim().toLowerCase();
  if (query === "") return [];
  const hits: Entity[] = [];
  for (const lot of lots()) {
    if (lot.id.toLowerCase().includes(query)) hits.push({ kind: "foup", lot_id: lot.id });
    for (const wafer of lot.wafers) {
      if (wafer.id.toLowerCase().includes(query)) hits.push({ kind: "wafer", lot_id: lot.id, wafer_id: wafer.id });
    }
  }
  for (const tool of tools()) {
    if (tool.id.toLowerCase().includes(query)) hits.push({ kind: "tool", equipment_id: tool.id });
  }
  return hits.slice(0, 6);
}

function SearchCard() {
  return (
    <View
      style={{
        width: "100%",
        padding: 12,
        gap: 8,
        borderRadius: 10,
        backgroundColor: p().surface,
        borderWidth: 1,
        borderColor: p().border,
      }}
    >
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Text style={{ fontSize: 13.5, fontWeight: "600", color: p().text }}>按 ID 定位</Text>
        <View style={{ flex: 1 }} />
        <Text style={{ fontSize: TINY, color: p().muted }}>lot / wafer / equipment / event_id</Text>
      </View>
      <Input
        value={search()}
        onChange={({ value }) => search.set(value)}
        placeholder="输入 ID，如 LOT-0001 或 event UUID"
        style={{
          paddingHorizontal: 8,
          paddingVertical: 4,
          borderRadius: 6,
          backgroundColor: p().surfaceAlt,
          color: p().text,
          fontSize: SMALL,
        }}
      />
      {search().trim() === "" ? null : (
        <View style={{ gap: 4 }}>
          <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 4 }}>
            {searchHits().map((entity) => (
              <View
                onPress={() => drill(entity)}
                style={{ paddingHorizontal: 6, paddingVertical: 2, borderRadius: 5, backgroundColor: p().surfaceAlt }}
              >
                <Text style={{ fontSize: TINY, color: p().info }}>{entityLabel(entity)}</Text>
              </View>
            ))}
          </View>
          {search().length > 30 && search().includes("-") ? (
            <Button label="按 event_id 追踪到 outbox 与数据点" accent onPress={() => void loadTrace(search())} />
          ) : null}
        </View>
      )}
    </View>
  );
}

function LegendCard() {
  return (
    <View
      style={{
        width: "100%",
        padding: 12,
        gap: 8,
        borderRadius: 10,
        backgroundColor: p().surface,
        borderWidth: 1,
        borderColor: p().border,
      }}
    >
      <Text style={{ fontSize: 13.5, fontWeight: "600", color: p().text }}>图例</Text>
      <Small text="同一语义在 3D 与表格里同色" />
      <View style={{ gap: 4 }}>
        {([
          ["运行 / 已完工", p().good],
          ["维护 / Hold", p().warn],
          ["停机 / 报废", p().bad],
          ["处理中", p().info],
          ["空闲 / 未开工", p().muted],
        ] as [string, string][]).map(([label, color]) => (
          <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
            <Dot color={color} />
            <Small text={label} />
          </View>
        ))}
      </View>
    </View>
  );
}

function DemoCard() {
  return (
    <View
      style={{
        width: "100%",
        padding: 12,
        gap: 8,
        borderRadius: 10,
        backgroundColor: p().surface,
        borderWidth: 1,
        borderColor: p().border,
      }}
    >
      <Text style={{ fontSize: 13.5, fontWeight: "600", color: p().text }}>引导演示</Text>
      <Small text="可随时中断，接管后不跳回" />
      <Small text={() => DEMO_LABEL[demo()] ?? "演示待启动"} />
      <View style={{ flexDirection: "row", gap: 4 }}>
        <Button label={() => (demoRunning() ? "停止" : "开始")} accent onPress={toggleDemo} />
        <Button label="下一步" onPress={startDemo} />
      </View>
    </View>
  );
}

function Toolbar() {
  return (
    <View style={{ flexDirection: "row", alignItems: "center", gap: 8, padding: 4 }}>
      <Button label="正交 / 透视" onPress={toggleProjection} />
      <Button label="机台聚焦" onPress={() => gotoLevel(1)} />
      <View style={{ flexDirection: "row", alignItems: "center", gap: 4 }}>
        <Small text={() => `爆炸 ${Math.round(explosion())}%`} />
        <Button label="−" onPress={() => explosion.set(Math.max(0, explosion() - 10))} />
        <Button label="+" onPress={() => explosion.set(Math.min(100, explosion() + 10))} />
      </View>
      <View style={{ flex: 1 }} />
      <Small text={levelLabel} />
    </View>
  );
}

function Center() {
  return (
    <View style={{ flex: 1, minWidth: 420, minHeight: 0, gap: 4 }}>
      <Toolbar />
      <ViewportPane />
      <View style={{ flexDirection: "row", alignItems: "center", gap: 12, paddingHorizontal: 4 }}>
        <Small text="拖拽旋转 · Shift+拖拽平移 · 滚轮缩放 · 点击拾取" />
        <View style={{ flex: 1 }} />
        {renderError() === null ? null : (
          <Text style={{ fontSize: TINY, color: p().bad }}>{renderError()}</Text>
        )}
        {adapter() === "" ? null : <Text style={{ fontSize: TINY, color: p().muted }}>{`wgpu · ${adapter()}`}</Text>}
      </View>
    </View>
  );
}

function Inspector() {
  return (
    <View style={{ width: INSPECTOR_WIDTH, minWidth: 360, minHeight: 0 }}>
      <Scroll style={{ flex: 1 }}>
        <View style={{ padding: 4, gap: 8 }}>
          <Small text={() => tabHint(tab())} />
          {tab() === "mes" ? (
            <MesPanel />
          ) : tab() === "quality" ? (
            <QualityPanel />
          ) : tab() === "supply" ? (
            <SupplyPanel />
          ) : tab() === "productivity" ? (
            <ProductivityPanel />
          ) : (
            <CrossPanel />
          )}
        </View>
      </Scroll>
    </View>
  );
}

function Rail() {
  return (
    <View style={{ width: RAIL_WIDTH, gap: 8 }}>
      <BreadcrumbCard />
      <SelectionCard />
      <SearchCard />
      <LegendCard />
      <DemoCard />
    </View>
  );
}

function Footer() {
  return (
    <View
      style={{
        width: "100%",
        paddingHorizontal: 12,
        paddingVertical: 4,
        flexDirection: "row",
        alignItems: "center",
        gap: 12,
        backgroundColor: p().surface,
        borderTopWidth: 1,
        borderColor: p().border,
      }}
    >
      <Small
        text={() =>
          `事件 ${eventsTotal()} · 已投递 ${publishedEvents()} · outbox 积压 ${outboxPending()} · Greptime ${
            greptimeOk() ? "在线" : "断流（积压在 PG）"
          }`
        }
      />
      <Small text={() => (traceEvent() === null ? "" : `L3 event ${traceEvent()}`)} />
      <View style={{ flex: 1 }} />
      {note() === null ? null : <Small text={() => note() ?? ""} />}
      {lastAction() === null ? null : <Small text={() => lastAction() ?? ""} />}
    </View>
  );
}

export function Shell() {
  return (
    <View style={{ flex: 1, minHeight: 0, backgroundColor: p().background }}>
      <Header />
      <View style={{ flex: 1, flexDirection: "row", minHeight: 0, gap: 8, paddingHorizontal: 8, paddingBottom: 8 }}>
        <Rail />
        <Center />
        <Inspector />
      </View>
      <Footer />
    </View>
  );
}
