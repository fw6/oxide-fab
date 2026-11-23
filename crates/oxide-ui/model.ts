/** The wire model, the event fold, and the formatting the panels share.
 *
 * Field names are the API's (snake_case); nothing is renamed on the way in so a
 * payload is readable beside the Rust types it came from. Timestamps are Unix
 * epoch milliseconds unless the field ends in `_at`, which is ISO text.
 */
import type { EntityInput } from "oxide-ui/native";
import { DEFECT, type Palette } from "./palette";

export type TabId = "mes" | "quality" | "productivity" | "supply" | "cross";

export interface StepView {
  step_no: number;
  name: string;
  equipment_kind: string;
  queue_len: number;
  busy_tools: number;
  total_tools: number;
  current_lots: string[];
}

export interface ToolView {
  id: string;
  kind: string;
  chamber: string;
  state: string;
  light: string;
  current_lot: string | null;
  current_step: number | null;
  lots_processed: number;
}

export interface WaferView {
  id: string;
  slot: number;
  state: string;
  defect_info: string | null;
  rework_count: number;
}

export interface LotView {
  id: string;
  product_id: string;
  state: string;
  current_step: number;
  priority: number;
  wafer_count: number;
  done_wafers: number;
  hold_reason: string | null;
  parent_lot_id: string | null;
  wafers: WaferView[];
}

export interface DispatchDecision {
  at_ms: number;
  lot_id: string;
  step_no: number;
  equipment_id: string;
  priority: number;
  queue_position: number;
  queue_len: number;
  qualified_tools: string[];
  recipe_id: string;
  rule: string;
}

export interface StepCycle {
  step_no: number;
  planned_ms: number;
  actual_mean_ms: number | null;
  samples: number;
}

export interface LotCycle {
  lot_id: string;
  created_ms: number;
  completed_ms: number;
  cycle_ms: number;
}

export interface ToolKpi {
  equipment_id: string;
  kind: string;
  busy_ms: number;
  down_ms: number;
  track_ins: number;
  track_outs: number;
  utilization: number;
  availability: number;
}

export interface DefectCount {
  class: string;
  count: number;
}

export interface MetricNote {
  metric: string;
  source: string;
  formula: string;
}

export interface Kpi {
  lots_created: number;
  lots_completed: number;
  wafers_created: number;
  wafers_scrapped: number;
  first_pass_yield: number | null;
  mean_cycle_time_ms: number | null;
  max_cycle_time_ms: number | null;
  throughput_lots_per_hour: number | null;
  wip_lots: number;
  window_start_ms: number | null;
  window_end_ms: number | null;
  tools: ToolKpi[];
  defects: DefectCount[];
  step_cycles: StepCycle[];
  lot_cycles: LotCycle[];
}

export interface RecipeCheck {
  recipe_id: string;
  version: number;
  checksum: string;
  steps: number[];
  runs: number;
  verified_runs: number;
  mismatched_runs: number;
  last_run_ms: number | null;
  status: "verified" | "mismatched" | "not_run";
}

export interface QTimeLimit {
  from_step: number;
  to_step: number;
  limit_ms: number;
}

export interface QTimeViolation {
  lot_id: string;
  wafer_ids: string[];
  from_step: number;
  to_step: number;
  elapsed_ms: number;
  limit_ms: number;
  occurred_at_ms: number;
  event_id: string;
  equipment_id: string;
}

export interface Alarm {
  event_id: string;
  equipment_id: string;
  kind: "down" | "up";
  at_ms: number;
  reason: string;
  down_ms: number;
}

export interface Analysis {
  recipes: RecipeCheck[];
  qtime_limits: QTimeLimit[];
  qtime_violations: QTimeViolation[];
  alarms: Alarm[];
  kpi: Kpi;
  notes: MetricNote[];
}

export interface SpcBucket {
  ts: string;
  equipment_id: string;
  param: string;
  avg_value: number;
  min_value: number;
  max_value: number;
  ucl: number;
  lcl: number;
}

export interface ChaosSample {
  at_ms: number;
  pending: number;
  published: number;
  oldest_pending_ms: number;
  delivered_rows: number;
}

export interface OutboxStats {
  pending: number;
  published: number;
  oldest_pending_ms: number;
  max_attempts: number;
}

export interface TimelineResponse {
  samples: ChaosSample[];
  stats: OutboxStats;
}

export interface SecsStatus {
  host_connected: boolean;
  equipment_online: boolean;
  s1f1_ok: number;
  s6f11_sent: number;
  s5f1_sent: number;
  s7f3_uploaded: number;
  s7f4_ack: number;
  s7f6_verified: number;
  spool_depth: number;
  spooled_total: number;
  spooled_replayed: number;
  link_drops: number;
  last_error: string | null;
  traffic: unknown[];
}

export interface BackupStatus {
  taken_at: string | null;
  pg_full_backup: string | null;
  pg_logical_dump: string | null;
  greptime_snapshot: string | null;
  wal_continuity: string | null;
  ok: boolean;
  log_path: string | null;
}

export interface RestoreStatus {
  drilled_at: string | null;
  target_time: string | null;
  lots: number | null;
  events: number | null;
  max_created_at: string | null;
  greptime_rows: number | null;
  elapsed_ms: number | null;
  ok: boolean;
  log_path: string | null;
}

export interface DbCounts {
  lots: number;
  wafers: number;
  events: number;
  outbox_pending: number;
  outbox_published: number;
  published_events: number;
}

export interface SimInfo {
  now_ms: number;
  running: boolean;
  speed: number;
  released_lots: number;
  total_lots: number;
  persisted_events: number;
  steps: StepView[];
  equipment: ToolView[];
  lots: LotView[];
  dispatch: DispatchDecision[];
}

export interface Snapshot {
  sim: SimInfo;
  db: DbCounts;
  outbox: OutboxStats;
  last_seq: number;
  greptime_ok: boolean;
  secs: SecsStatus | null;
  backup: BackupStatus;
  restore: RestoreStatus | null;
  spc: SpcBucket[];
  spc_source: string;
  chaos: ChaosSample[];
}

export interface ComponentHealth {
  name: string;
  status: string;
  detail: string;
  latency_ms: number;
}

export interface MonitorResponse {
  components: ComponentHealth[];
  checked_at_ms: number;
}

export interface OutboxTrace {
  event_id: string;
  topic: string;
  attempts: number;
  created_at: string;
  published_at: string | null;
  last_error: string | null;
}

export interface Event {
  event_id: string;
  lot_id: string;
  wafer_id: string | null;
  kind: string;
  payload: Record<string, unknown>;
  occurred_at_ms: number;
  actor: string;
}

export interface PersistedEvent {
  seq: number;
  event: Event;
}

export interface EventTrace {
  event_id: string;
  event: PersistedEvent;
  outbox: OutboxTrace | null;
  greptime: Record<string, unknown[]>;
}

export interface EventsResponse {
  events: PersistedEvent[];
  last_seq: number;
}

export interface StepRecord {
  step_no: number;
  equipment_id: string;
  recipe_id: string;
  track_in_ms: number;
  track_out_ms: number;
  duration_ms: number;
  measured: number | null;
}

export interface WaferGenealogy {
  wafer_id: string;
  lot_id: string;
  slot: number;
  state: string;
  defect_info: string | null;
  rework_count: number;
  steps: StepRecord[];
}

export interface LotDetail {
  lot: LotView;
  wafers: WaferView[];
  genealogy: WaferGenealogy[];
  events: PersistedEvent[];
}

export interface WhatIfRun {
  label: string;
  rule: string;
  added_tool: string | null;
  completed_lots: number;
  mean_cycle_time_ms: number | null;
  throughput_lots_per_hour: number | null;
  wip_end: number;
  simulated_ms: number;
  timed_out: boolean;
  from_start: boolean;
}

export interface WhatIfDelta {
  completed_lots: number;
  mean_cycle_time_ms: number | null;
  throughput_lots_per_hour: number | null;
  wip_end: number;
}

export interface WhatIfComparison {
  baseline: WhatIfRun;
  variant: WhatIfRun;
  delta: WhatIfDelta;
  notes: string[];
}

/** A scene entity, the same shape the viewport picks and TSX names. */
export type Entity =
  | { kind: "floor" }
  | { kind: "band"; step_no: number }
  | { kind: "rail" }
  | { kind: "tool"; equipment_id: string }
  | { kind: "toolPart"; equipment_id: string; part: string }
  | { kind: "foup"; lot_id: string }
  | { kind: "wafer"; lot_id: string; wafer_id: string }
  | { kind: "exit" };

export function entityLabel(entity: Entity): string {
  switch (entity.kind) {
    case "floor":
      return "厂区地面";
    case "band":
      return `工序带 S${entity.step_no}`;
    case "rail":
      return "AMHS 环线";
    case "tool":
      return entity.equipment_id;
    case "toolPart":
      return `${entity.equipment_id} · ${entity.part}`;
    case "foup":
      return `${entity.lot_id} 载具`;
    case "wafer":
      return `${entity.lot_id} · ${entity.wafer_id}`;
    case "exit":
      return "出片区";
  }
}

export function toolIdOf(entity: Entity): string | null {
  return entity.kind === "tool" || entity.kind === "toolPart" ? entity.equipment_id : null;
}

export function lotIdOf(entity: Entity): string | null {
  return entity.kind === "foup" || entity.kind === "wafer" ? entity.lot_id : null;
}

export function toEntityInput(entity: Entity | null): EntityInput | null {
  if (entity === null) return null;
  switch (entity.kind) {
    case "floor":
    case "rail":
    case "exit":
      return { kind: entity.kind };
    case "band":
      return { kind: "band", stepNo: entity.step_no };
    case "tool":
      return { kind: "tool", equipmentId: entity.equipment_id };
    case "toolPart":
      return { kind: "toolPart", equipmentId: entity.equipment_id, part: entity.part };
    case "foup":
      return { kind: "foup", lotId: entity.lot_id };
    case "wafer":
      return { kind: "wafer", lotId: entity.lot_id, waferId: entity.wafer_id };
  }
}

export function fromEntityInput(input: EntityInput | null): Entity | null {
  if (input === null) return null;
  switch (input.kind) {
    case "floor":
    case "rail":
    case "exit":
      return { kind: input.kind };
    case "band":
      return input.stepNo === null || input.stepNo === undefined
        ? null
        : { kind: "band", step_no: input.stepNo };
    case "tool":
      return input.equipmentId ? { kind: "tool", equipment_id: input.equipmentId } : null;
    case "toolPart":
      return input.equipmentId && input.part
        ? { kind: "toolPart", equipment_id: input.equipmentId, part: input.part }
        : null;
    case "foup":
      return input.lotId ? { kind: "foup", lot_id: input.lotId } : null;
    case "wafer":
      return input.lotId && input.waferId
        ? { kind: "wafer", lot_id: input.lotId, wafer_id: input.waferId }
        : null;
  }
}

/** When each lot entered its current step, for the FOUP transit. */
export type StepSince = Record<string, [number, number]>;

/** Records the step-entry time for every lot whose step changed. */
export function noteSteps(
  steps: StepSince,
  previous: LotView[],
  next: LotView[],
  nowMs: number,
): { steps: StepSince; changed: boolean } {
  let changed = false;
  const out: StepSince = { ...steps };
  for (const lot of next) {
    const before = previous.find((old) => old.id === lot.id);
    if (before === undefined || before.current_step !== lot.current_step) {
      out[lot.id] = [lot.current_step, nowMs];
      changed = true;
    }
  }
  return { steps: out, changed };
}

/** Folds one event page into the lot and tool projections.
 *
 * `true` means the fold applied (or the kind is deliberately ignored); `false`
 * means the view does not know the lot or tool, so a full snapshot is needed.
 */
export function applyEvent(lots: LotView[], tools: ToolView[], event: Event): boolean {
  const payload = event.payload as Record<string, unknown>;
  const stepNo = typeof payload.step_no === "number" ? payload.step_no : undefined;
  const equipmentId = typeof payload.equipment_id === "string" ? payload.equipment_id : undefined;
  const wafers = Array.isArray(payload.wafers)
    ? payload.wafers.filter((wafer): wafer is string => typeof wafer === "string")
    : [];
  const lot = lots.find((candidate) => candidate.id === event.lot_id);

  if (event.kind === "lot_created") return false;
  if (event.kind === "equipment_down" || event.kind === "equipment_up") {
    if (equipmentId === undefined) return false;
    const tool = tools.find((candidate) => candidate.id === equipmentId);
    if (tool === undefined) return false;
    if (event.kind === "equipment_down") {
      tool.state = "down";
      tool.light = "red";
    } else {
      tool.state = "idle";
      tool.light = "green";
    }
    return true;
  }

  switch (event.kind) {
    case "track_in": {
      if (lot === undefined) return false;
      lot.state = "processing";
      if (stepNo !== undefined) lot.current_step = stepNo;
      for (const wafer of lot.wafers) {
        if (wafers.includes(wafer.id)) wafer.state = "processing";
      }
      const tool = tools.find((candidate) => candidate.id === equipmentId);
      if (tool !== undefined) {
        tool.state = "running";
        tool.light = "green";
        tool.current_lot = lot.id;
        tool.current_step = stepNo ?? null;
      }
      return true;
    }
    case "track_out": {
      if (lot === undefined) return false;
      for (const wafer of lot.wafers) {
        if (wafers.includes(wafer.id)) wafer.state = "waiting";
      }
      lot.state = "waiting";
      const tool = tools.find((candidate) => candidate.id === equipmentId);
      if (tool !== undefined) {
        tool.state = "idle";
        tool.light = "green";
        tool.current_lot = null;
        tool.current_step = null;
        tool.lots_processed += 1;
      }
      return true;
    }
    case "lot_queued": {
      if (lot === undefined) return false;
      lot.state = "queued";
      if (stepNo !== undefined) lot.current_step = stepNo;
      return true;
    }
    case "lot_completed": {
      if (lot === undefined) return false;
      lot.state = "completed";
      lot.done_wafers = lot.wafer_count;
      for (const wafer of lot.wafers) {
        if (wafer.state !== "scrapped") wafer.state = "done";
      }
      return true;
    }
    case "lot_held": {
      if (lot === undefined) return false;
      lot.state = "on_hold";
      lot.hold_reason = typeof payload.reason === "string" ? payload.reason : null;
      return true;
    }
    case "lot_released": {
      if (lot === undefined) return false;
      lot.state = "waiting";
      lot.hold_reason = null;
      return true;
    }
    case "rework_started": {
      if (lot === undefined) return false;
      lot.state = "waiting";
      if (stepNo !== undefined) lot.current_step = stepNo;
      for (const wafer of lot.wafers) {
        if (wafer.state !== "scrapped") {
          wafer.state = "waiting";
          wafer.rework_count += 1;
        }
      }
      return true;
    }
    case "wafer_scrapped": {
      if (lot === undefined) return false;
      const waferId = typeof payload.wafer_id === "string" ? payload.wafer_id : undefined;
      const wafer = lot.wafers.find((candidate) => candidate.id === waferId);
      if (wafer !== undefined) {
        wafer.state = "scrapped";
        wafer.defect_info = typeof payload.reason === "string" ? payload.reason : null;
      }
      return true;
    }
    case "track_out_duplicate":
      return lot !== undefined;
    default:
      return true;
  }
}

/** `HH:MM:SS UTC` of a simulation timestamp. */
export function clockText(nowMs: number): string {
  const seconds = Math.floor(nowMs / 1000);
  const today = ((seconds % 86_400) + 86_400) % 86_400;
  const hh = String(Math.floor(today / 3600)).padStart(2, "0");
  const mm = String(Math.floor((today % 3600) / 60)).padStart(2, "0");
  const ss = String(today % 60).padStart(2, "0");
  return `${hh}:${mm}:${ss} UTC`;
}

/** `MM:SS`, minutes uncapped. */
export function durationText(ms: number): string {
  const total = Math.round(Math.max(0, ms) / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

export function stateColor(palette: Palette, state: string): string {
  switch (state) {
    case "completed":
      return palette.good;
    case "processing":
      return palette.info;
    case "on_hold":
      return palette.warn;
    case "split":
    case "merged":
      return palette.muted;
    default:
      return palette.border;
  }
}

export function lightColor(palette: Palette, light: string): string {
  if (light === "red") return palette.bad;
  if (light === "amber") return palette.warn;
  return palette.good;
}

type DefectClass = "scratch" | "particle" | "resist" | "pattern" | "other";

function defectClass(defect: string | null): DefectClass | null {
  if (defect === null || defect === "") return null;
  const text = defect.toLowerCase();
  if (text.includes("scratch")) return "scratch";
  if (text.includes("particle")) return "particle";
  if (text.includes("resist")) return "resist";
  if (text.includes("pattern") || text.includes("cd") || text.includes("defocus")) return "pattern";
  return "other";
}

export function defectFill(palette: Palette, defect: string | null): string | null {
  const kind = defectClass(defect);
  if (kind === null) return null;
  if (kind === "other") return palette.bad;
  return DEFECT[kind];
}
