/** The one shared state: signals, the poller, and every action a panel offers.
 *
 * Nothing caches per panel. The snapshot owns aggregates, the clock and flags;
 * an event page folds into `lots` and `tools` only, and anything it cannot fold
 * reloads the snapshot. Errors only ever land in `note` and never stop the poll.
 */
import { Clipboard, signal } from "nsh";
import { Desktop, Viewport } from "oxide-ui/native";
import {
  applyEvent,
  type Analysis,
  type BackupStatus,
  type ChaosSample,
  type DispatchDecision,
  type Entity,
  type EventTrace,
  type EventsResponse,
  type LotDetail,
  type LotView,
  type MonitorResponse,
  type PersistedEvent,
  type RestoreStatus,
  type SecsStatus,
  type Snapshot,
  type SpcBucket,
  type StepView,
  type TabId,
  type ToolView,
  type WhatIfComparison,
  entityLabel,
  lotIdOf,
  noteSteps,
  toEntityInput,
  toolIdOf,
  type StepSince,
} from "./model";

export const viewport = Viewport;

const API_URL = "http://127.0.0.1:18080";

function apiBase(): string {
  const configured = process.env.OXIDE_API_URL;
  const base = configured !== undefined && configured !== "" ? configured : API_URL;
  return base.endsWith("/") ? base.slice(0, -1) : base;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function getJson<T>(path: string): Promise<T> {
  const response = await fetch(`${apiBase()}${path}`);
  if (!response.ok) throw new Error(`HTTP ${response.status} ${path}`);
  return (await response.json()) as T;
}

async function postJson<T>(path: string, body: unknown): Promise<T> {
  const response = await fetch(`${apiBase()}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!response.ok) throw new Error(`HTTP ${response.status} ${path}`);
  return (await response.json()) as T;
}

// ── Signals ────────────────────────────────────────────────────────────────

export const steps = signal<StepView[]>([]);
export const tools = signal<ToolView[]>([]);
export const lots = signal<LotView[]>([]);
export const spc = signal<SpcBucket[]>([]);
export const spcSource = signal("raw");
export const chaos = signal<ChaosSample[]>([]);
export const secs = signal<SecsStatus | null>(null);
export const backup = signal<BackupStatus | null>(null);
export const restore = signal<RestoreStatus | null>(null);
export const outboxPending = signal(0);
export const outboxPublished = signal(0);
export const eventsTotal = signal(0);
export const publishedEvents = signal(0);
export const nowMs = signal(0);
export const running = signal(false);
export const speed = signal(10);
export const lastSeq = signal(0);
export const connected = signal(false);
export const greptimeOk = signal(false);
export const note = signal<string | null>(null);
export const stepSince = signal<StepSince>({});
export const dispatch = signal<DispatchDecision[]>([]);

export const level = signal<"factory" | "tool" | "lot" | "event">("factory");
export const focus = signal<Entity | null>(null);
export const selected = signal<Entity | null>(null);
export const trail = signal<Entity[]>([]);
export const explosion = signal(0);
export const reduceMotion = signal(false);
export const projection = signal<"perspective" | "orthographic">("perspective");
export const dark = signal(true);
export const adapter = signal("");
export const renderError = signal<string | null>(null);

export const selectedLot = signal<string | null>(null);
export const selectedWafer = signal<string | null>(null);
export const detail = signal<LotDetail | null>(null);
export const trace = signal<EventTrace | null>(null);
export const traceEvent = signal<string | null>(null);
export const analysis = signal<Analysis | null>(null);
export const monitor = signal<MonitorResponse | null>(null);
export const whatif = signal<WhatIfComparison | null>(null);
export const whatifFifo = signal(false);
export const whatifAddTool = signal(true);
export const whatifBusy = signal(false);

export const tab = signal<TabId>("mes");
export const search = signal("");
export const dense = signal(true);

export const demo = signal("idle");
export const demoGen = signal(0);
export const backend = signal("未检查");
export const lastAction = signal<string | null>(null);

export const TAB_ORDER: TabId[] = ["mes", "quality", "productivity", "supply", "cross"];

export function tabLabel(id: TabId): string {
  switch (id) {
    case "mes":
      return "制造执行";
    case "quality":
      return "工艺质量";
    case "productivity":
      return "生产效率";
    case "supply":
      return "供应链";
    case "cross":
      return "横切 · 总线与 KPI";
  }
}

export function tabHint(id: TabId): string {
  switch (id) {
    case "mes":
      return "MES 300Works 语义：WIP、genealogy、载具";
    case "quality":
      return "Applied E3 语义：SPC、FD、RMS、Q-time、良率";
    case "productivity":
      return "APF 语义：RTD、派工、WIP、what-if";
    case "supply":
      return "计划与实际节拍、交付偏差预警";
    case "cross":
      return "Monitor、outbox、KPI、演练与导出";
  }
}

const LEVEL_LABEL: Record<"factory" | "tool" | "lot" | "event", string> = {
  factory: "L0 全厂",
  tool: "L1 机台",
  lot: "L2 批次",
  event: "L3 事件",
};

export function levelLabel(): string {
  const current = focus();
  return `${LEVEL_LABEL[level()]} · ${current === null ? "全厂" : entityLabel(current)}`;
}

export function latestLot(): LotView | null {
  const id = selectedLot();
  return id === null ? null : (lots().find((lot) => lot.id === id) ?? null);
}

// ── Loaders ────────────────────────────────────────────────────────────────

export async function loadSnapshot(): Promise<void> {
  const snapshot = await getJson<Snapshot>("/api/snapshot");
  const previous = lots();
  const noted = noteSteps(stepSince(), previous, snapshot.sim.lots, snapshot.sim.now_ms);
  if (noted.changed) stepSince.set(noted.steps);
  steps.set(snapshot.sim.steps);
  tools.set(snapshot.sim.equipment);
  lots.set(snapshot.sim.lots);
  dispatch.set(snapshot.sim.dispatch);
  spc.set(snapshot.spc);
  spcSource.set(snapshot.spc_source === "" ? "raw" : snapshot.spc_source);
  chaos.set(snapshot.chaos);
  secs.set(snapshot.secs);
  backup.set(snapshot.backup.taken_at === null ? null : snapshot.backup);
  restore.set(snapshot.restore);
  outboxPending.set(snapshot.db.outbox_pending);
  outboxPublished.set(snapshot.db.outbox_published);
  eventsTotal.set(snapshot.db.events);
  publishedEvents.set(snapshot.db.published_events);
  nowMs.set(snapshot.sim.now_ms);
  running.set(snapshot.sim.running);
  speed.set(snapshot.sim.speed);
  lastSeq.set(snapshot.last_seq);
  greptimeOk.set(snapshot.greptime_ok);
  connected.set(true);
  applyStartupView();
}

export async function loadAnalytics(): Promise<void> {
  analysis.set(await getJson<Analysis>("/api/analytics"));
}

export async function loadMonitor(): Promise<void> {
  monitor.set(await getJson<MonitorResponse>("/api/monitor"));
}

export async function loadLot(lotId: string): Promise<void> {
  selectedLot.set(lotId);
  selectedWafer.set(null);
  try {
    detail.set(await getJson<LotDetail>(`/api/lots/${encodeURIComponent(lotId)}`));
  } catch (error) {
    note.set(message(error));
  }
}

export async function loadTrace(eventId: string): Promise<void> {
  traceEvent.set(eventId);
  try {
    const loaded = await getJson<EventTrace>(`/api/events/${encodeURIComponent(eventId)}`);
    const payload = loaded.event.event.payload as Record<string, unknown>;
    const equipmentId = typeof payload.equipment_id === "string" ? payload.equipment_id : null;
    let subject: Entity | null = null;
    if (equipmentId !== null) subject = { kind: "tool", equipment_id: equipmentId };
    else if (loaded.event.event.lot_id !== "") subject = { kind: "foup", lot_id: loaded.event.event.lot_id };
    trace.set(loaded);
    level.set("event");
    tab.set("quality");
    if (subject !== null) {
      focus.set(subject);
      selected.set(subject);
      viewport.focus(toEntityInput(subject));
    }
    void loadAnalytics().catch(() => undefined);
  } catch (error) {
    note.set(`事件 ${eventId}: ${message(error)}`);
  }
}

/** One event page; `false` means the fold needs a fresh snapshot. */
async function pullEvents(): Promise<boolean> {
  const page = await getJson<EventsResponse>(`/api/events?since=${lastSeq()}&limit=500`);
  lastSeq.set(page.last_seq);
  if (page.events.length === 0) return true;
  const nextLots = lots().map((lot) => ({ ...lot, wafers: lot.wafers.map((wafer) => ({ ...wafer })) }));
  const nextTools = tools().map((tool) => ({ ...tool }));
  let folded = true;
  for (const entry of page.events) {
    if (!applyEvent(nextLots, nextTools, entry.event)) {
      folded = false;
      break;
    }
  }
  if (!folded) return false;
  const last = page.events.at(-1);
  const at = last === undefined ? nowMs() : last.event.occurred_at_ms;
  const noted = noteSteps(stepSince(), lots(), nextLots, at);
  if (noted.changed) stepSince.set(noted.steps);
  lots.set(nextLots);
  tools.set(nextTools);
  return true;
}

// ── Actions ────────────────────────────────────────────────────────────────

async function action(path: string, body: unknown, label: string): Promise<void> {
  if (!connected()) {
    note.set("后端未就绪：写操作已禁用，只读引导仍可用");
    return;
  }
  try {
    await postJson(path, body);
    lastAction.set(`${label} · 已写入 lot_event`);
    await pullEvents();
    await loadSnapshot();
    await loadAnalytics();
    const lot = selectedLot();
    if (lot !== null) await loadLot(lot);
  } catch (error) {
    note.set(`${label}: ${message(error)}`);
  }
}

export function sim(path: string, label: string): void {
  void action(path, {}, label);
}

export function hold(): void {
  const lot = selectedLot();
  if (lot !== null) void action(`/api/lots/${encodeURIComponent(lot)}/hold`, { reason: "workbench hold" }, "Hold");
}

export function release(): void {
  const lot = selectedLot();
  if (lot !== null) void action(`/api/lots/${encodeURIComponent(lot)}/release`, {}, "Release");
}

export function rework(): void {
  const lot = latestLot();
  if (lot === null) return;
  void action(
    `/api/lots/${encodeURIComponent(lot.id)}/rework`,
    { to_step: Math.max(lot.current_step - 1, 1), reason: "workbench rework" },
    "Rework",
  );
}

export function splitLot(): void {
  const lot = selectedLot();
  if (lot === null) return;
  void action(`/api/lots/${encodeURIComponent(lot)}/split`, { children: [`${lot}-A`, `${lot}-B`] }, "Split");
}

export function mergeLot(): void {
  const target = latestLot();
  if (target === null) return;
  const source = lots().find(
    (lot) =>
      lot.id !== target.id &&
      lot.current_step === target.current_step &&
      !["completed", "split", "merged"].includes(lot.state),
  );
  if (source === undefined) {
    note.set("没有同工序可合并的批次");
    return;
  }
  void action(`/api/lots/${encodeURIComponent(target.id)}/merge`, { sources: [source.id] }, "Merge");
}

export function scrapWafer(): void {
  const loaded = detail();
  if (loaded === null) return;
  const wafer = loaded.wafers.find((candidate) => candidate.state !== "scrapped");
  if (wafer === undefined) return;
  void action(
    `/api/lots/${encodeURIComponent(loaded.lot.id)}/scrap`,
    { wafer_id: wafer.id, reason: "workbench scrap" },
    "Scrap",
  );
}

export async function runWhatIf(): Promise<void> {
  const current = focus();
  const tool = current === null ? null : toolIdOf(current);
  const kind = tools().find((candidate) => candidate.id === tool)?.kind ?? "etch";
  whatifBusy.set(true);
  try {
    const comparison = await postJson<WhatIfComparison>("/api/what-if", {
      rule: whatifFifo() ? "fifo" : "priority",
      add_tool_kind: whatifAddTool() ? kind : null,
      horizon_ms: 7_200_000,
    });
    whatif.set(comparison);
    lastAction.set("what-if 完成：当前运行实例未被改写");
  } catch (error) {
    note.set(`what-if: ${message(error)}`);
  } finally {
    whatifBusy.set(false);
  }
}

async function save(suggestedName: string, contents: string): Promise<void> {
  try {
    const path = await Desktop.saveJson(suggestedName, contents);
    lastAction.set(`已导出 ${path}`);
  } catch (error) {
    const text = message(error);
    if (text !== "已取消") note.set(text);
  }
}

export function exportScene(): void {
  void save("oxide-scene.json", viewport.describe());
}

export function exportLot(): void {
  const loaded = detail();
  if (loaded === null) {
    note.set("先选中一个批次");
    return;
  }
  void save(
    `${loaded.lot.id}.json`,
    JSON.stringify(
      {
        lot_id: loaded.lot.id,
        state: loaded.lot.state,
        current_step: loaded.lot.current_step,
        wafers: loaded.wafers,
        genealogy: loaded.genealogy,
        events: loaded.events,
        source: "lot_event（按 seq 完整）、wafer 投影、genealogy 推导",
      },
      null,
      2,
    ),
  );
}

export async function exportChaos(): Promise<void> {
  const [timeline, analytics, backupStatus, restoreStatus] = await Promise.allSettled([
    getJson<unknown>("/api/outbox/timeline"),
    getJson<unknown>("/api/analytics"),
    getJson<unknown>("/api/backup"),
    getJson<unknown>("/api/restore"),
  ]);
  const value = (result: PromiseSettledResult<unknown>) =>
    result.status === "fulfilled" ? result.value : null;
  void save(
    "oxide-chaos-report.json",
    JSON.stringify(
      {
        exported_at_ms: nowMs(),
        timeline: value(timeline),
        analytics: value(analytics),
        backup: value(backupStatus),
        restore: value(restoreStatus),
        rpo_rto_note: "RPO/RTO 来自 scripts/backup.sh、restore.sh、chaos.sh 的产物；UI 只读取并展示。",
      },
      null,
      2,
    ),
  );
}

export async function startBackend(): Promise<void> {
  backend.set("启动中…");
  try {
    backend.set(await Desktop.startBackend(process.env.OXIDE_REPO_ROOT ?? "."));
    await Promise.allSettled([loadSnapshot(), loadAnalytics(), loadMonitor()]);
  } catch (error) {
    backend.set(message(error));
  }
}

export function copyTraceEvent(): void {
  const id = traceEvent();
  if (id !== null) void Clipboard.writeText(id);
}

// ── Navigation ─────────────────────────────────────────────────────────────

export function select(entity: Entity | null): void {
  selected.set(entity);
}

export function drill(entity: Entity): void {
  selected.set(entity);
  const tool = toolIdOf(entity);
  const lot = lotIdOf(entity);
  if (tool !== null) {
    const target: Entity = { kind: "tool", equipment_id: tool };
    level.set("tool");
    focus.set(target);
    trail.update((items) => [...items, entity]);
    viewport.focus(toEntityInput(target));
  } else if (lot !== null) {
    const target: Entity = { kind: "foup", lot_id: lot };
    level.set("lot");
    focus.set(target);
    trail.update((items) => [...items, entity]);
    void loadLot(lot);
    if (entity.kind === "wafer") selectedWafer.set(entity.wafer_id);
    viewport.focus(toEntityInput(target));
  } else {
    level.set("factory");
    focus.set(null);
    viewport.focus(null);
  }
}

export function levelUp(): void {
  const items = trail();
  const previous = items.at(-1);
  trail.set(items.slice(0, -1));
  if (previous === undefined) {
    level.set("factory");
    focus.set(null);
    viewport.focus(null);
    return;
  }
  if (previous.kind === "tool" || previous.kind === "toolPart") level.set("tool");
  else if (previous.kind === "foup" || previous.kind === "wafer") level.set("lot");
  else {
    level.set("factory");
    focus.set(null);
    viewport.focus(null);
    return;
  }
  focus.set(previous);
  viewport.focus(toEntityInput(previous));
}

export function fit(): void {
  level.set("factory");
  focus.set(null);
  trail.set([]);
  viewport.resetCamera();
}

/** Switches the camera between perspective and orthographic. */
export function toggleProjection(): void {
  projection.set(viewport.toggleProjection() === "orthographic" ? "orthographic" : "perspective");
}

export function gotoLevel(target: 0 | 1 | 2 | 3): void {
  stopDemo();
  switch (target) {
    case 0:
      fit();
      return;
    case 1: {
      const current = focus();
      const tool = (current === null ? null : toolIdOf(current)) ?? tools()[0]?.id ?? null;
      if (tool === null) note.set("没有可进入的机台");
      else drill({ kind: "tool", equipment_id: tool });
      return;
    }
    case 2: {
      const lot = selectedLot() ?? lots()[0]?.id ?? null;
      if (lot === null) note.set("没有可进入的批次");
      else drill({ kind: "foup", lot_id: lot });
      return;
    }
    default: {
      const event =
        analysis()?.qtime_violations[0]?.event_id ??
        analysis()?.alarms[0]?.event_id ??
        traceEvent();
      if (event === null || event === undefined) note.set("尚无可追踪事件：在左侧输入一个 event_id");
      else void loadTrace(event);
    }
  }
}

// ── Demo ───────────────────────────────────────────────────────────────────

export const DEMO_LABEL: Record<string, string> = {
  idle: "演示待启动",
  factory: "1/8 全厂 · MES",
  tool: "2/8 机台拆解 · L1",
  lot: "3/8 批次逐片 · L2",
  event: "4/8 事件追踪 · L3",
  quality: "5/8 工艺质量",
  whatif: "6/8 离线 what-if",
  bus: "7/8 消息总线 chaos",
  kpi: "8/8 KPI 与 RPO/RTO",
  done: "演示结束：可继续操作",
};

export function demoRunning(): boolean {
  return demo() !== "idle";
}

export function stopDemo(): void {
  demoGen.update((generation) => generation + 1);
  demo.set("idle");
}

export function toggleDemo(): void {
  if (demoRunning()) stopDemo();
  else startDemo();
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export function startDemo(): void {
  const generation = demoGen() + 1;
  demoGen.set(generation);
  const alive = () => demoGen() === generation;
  void (async () => {
    demo.set("factory");
    tab.set("mes");
    explosion.set(0);
    fit();
    note.set("演示：全厂归属 MES；机台灯号与队列同源。");
    await sleep(2400);
    if (!alive()) return;

    const tool = tools()[0];
    if (tool !== undefined) {
      demo.set("tool");
      drill({ kind: "tool", equipment_id: tool.id });
      explosion.set(100);
      note.set("演示：部件按 manifest 的爆炸方向实时平移，未预烘焙任何图。");
    }
    await sleep(2800);
    if (!alive()) return;

    const lot = lots().find((candidate) => candidate.wafers.length > 0) ?? lots()[0];
    if (lot !== undefined) {
      demo.set("lot");
      drill({ kind: "foup", lot_id: lot.id });
      note.set("演示：25 槽网格按 wafer.state 与 defect_info 着色，逐片可下钻。");
    }
    await sleep(2800);
    if (!alive()) return;

    demo.set("event");
    const current = analysis();
    const event =
      current?.qtime_violations[0]?.event_id ??
      current?.alarms[0]?.event_id ??
      detail()?.events.at(-1)?.event.event_id;
    if (event === undefined) note.set("演示：尚无告警或 Q-time 事件，L3 由搜索任意 event_id 进入。");
    else await loadTrace(event);
    tab.set("quality");
    await sleep(2800);
    if (!alive()) return;

    demo.set("quality");
    tab.set("quality");
    note.set("演示：SPC、告警、配方校验、Q-time 与良率都来自同一份事件日志。");
    await sleep(2600);
    if (!alive()) return;

    demo.set("whatif");
    tab.set("productivity");
    void runWhatIf();
    note.set("演示：what-if 在隔离副本上重放当前 snapshot，运行实例不受影响。");
    await sleep(3200);
    if (!alive()) return;

    demo.set("bus");
    tab.set("cross");
    note.set("演示：outbox 积压与追平来自 PG；chaos 由 ./scripts/chaos.sh 执行，面板实时反映。");
    await sleep(2600);
    if (!alive()) return;

    demo.set("kpi");
    tab.set("cross");
    note.set("演示：KPI 每个数字都标注口径；RPO/RTO 取演练产物。");
    await sleep(2400);
    if (!alive()) return;

    demo.set("done");
  })();
}

// ── Startup ────────────────────────────────────────────────────────────────

let startupApplied = false;

function applyStartupView(): void {
  if (startupApplied || tools().length === 0) return;
  startupApplied = true;
  if (process.env.OXIDE_THEME === "light") dark.set(false);
  const explode = Number.parseFloat(process.env.OXIDE_EXPLODE);
  if (Number.isFinite(explode)) explosion.set(Math.min(100, Math.max(0, explode)));
  const wanted = process.env.OXIDE_TAB;
  if (wanted === "quality" || wanted === "productivity" || wanted === "supply" || wanted === "cross") {
    tab.set(wanted);
  }
  if (process.env.OXIDE_WHATIF === "1") void runWhatIf();
  const view = process.env.OXIDE_VIEW ?? "";
  if (view === "factory") fit();
  else if (view.startsWith("tool:")) drill({ kind: "tool", equipment_id: view.slice(5) });
  else if (view.startsWith("lot:")) drill({ kind: "foup", lot_id: view.slice(4) });
  else if (view.startsWith("event:")) void loadTrace(view.slice(6));
}

/** Seeds the signals and starts the 700 ms poll. Returns the stop function. */
export function bootstrap(): () => void {
  void loadSnapshot().catch(() => undefined);
  void loadAnalytics().catch(() => undefined);
  void loadMonitor().catch(() => undefined);
  if (process.env.OXIDE_DEMO === "1") {
    setTimeout(() => startDemo(), 800);
  }
  let ticks = 0;
  const id = setInterval(() => {
    void (async () => {
      try {
        const folded = await pullEvents();
        if (!folded) await loadSnapshot().catch(() => undefined);
        connected.set(true);
        if ((note() ?? "").startsWith("连接中断")) note.set(null);
      } catch (error) {
        connected.set(false);
        note.set(`连接中断，正在重连：${message(error)}`);
      }
    })();
    ticks += 1;
    if (ticks % 6 === 0) void loadSnapshot().catch(() => undefined);
    if (ticks % 9 === 0) {
      void loadAnalytics().catch(() => undefined);
      void loadMonitor().catch(() => undefined);
    }
  }, 700);
  return () => clearInterval(id);
}
