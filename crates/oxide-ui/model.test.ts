// The event fold and the clock were Rust unit tests before the interface moved
// to TSX (ADR 010); these are the same five, against the same functions.
import { expect, test } from "nsh/test";
import { applyEvent, clockText, liveNow, noteSteps, type Event, type LotView, type ToolView } from "./model";

function lot(): LotView {
  return {
    id: "LOT-0001",
    product_id: "PRD-OXIDE-300",
    state: "queued",
    current_step: 1,
    priority: 0,
    wafer_count: 2,
    done_wafers: 0,
    hold_reason: null,
    parent_lot_id: null,
    wafers: [
      { id: "LOT-0001-W01", slot: 1, state: "queued", defect_info: null, rework_count: 0 },
      { id: "LOT-0001-W02", slot: 2, state: "queued", defect_info: null, rework_count: 0 },
    ],
  };
}

function tool(): ToolView {
  return {
    id: "CLN-01",
    kind: "clean",
    chamber: "C1",
    state: "idle",
    light: "green",
    current_lot: null,
    current_step: null,
    lots_processed: 0,
  };
}

function event(kind: string, payload: Record<string, unknown>): Event {
  return {
    event_id: "00000000-0000-0000-0000-000000000001",
    lot_id: "LOT-0001",
    wafer_id: null,
    kind,
    payload,
    occurred_at_ms: 0,
    actor: "test",
  };
}

test("a track-in turns one tool light and the lot processing", () => {
  const lots = [lot()];
  const tools = [tool()];
  const ok = applyEvent(
    lots,
    tools,
    event("track_in", {
      step_no: 1,
      equipment_id: "CLN-01",
      wafers: ["LOT-0001-W01", "LOT-0001-W02"],
    }),
  );
  expect(ok).toBe(true);
  expect(lots[0].state).toBe("processing");
  expect(tools[0].state).toBe("running");
  expect(tools[0].current_lot).toBe("LOT-0001");
});

test("a track-out frees the tool and only touches its wafers", () => {
  const lots = [lot()];
  const tools = [tool()];
  applyEvent(
    lots,
    tools,
    event("track_in", { step_no: 1, equipment_id: "CLN-01", wafers: ["LOT-0001-W01"] }),
  );
  applyEvent(
    lots,
    tools,
    event("track_out", {
      step_no: 1,
      equipment_id: "CLN-01",
      wafers: ["LOT-0001-W01"],
      duration_ms: 6000,
    }),
  );
  expect(tools[0].state).toBe("idle");
  expect(tools[0].lots_processed).toBe(1);
  expect(lots[0].wafers[0].state).toBe("waiting");
  expect(lots[0].wafers[1].state).toBe("queued");
});

test("a hold survives the step it interrupted and release requeues the lot", () => {
  const lots = [lot()];
  const tools = [tool()];
  applyEvent(
    lots,
    tools,
    event("track_in", {
      step_no: 1,
      equipment_id: "CLN-01",
      wafers: ["LOT-0001-W01", "LOT-0001-W02"],
    }),
  );
  applyEvent(lots, tools, event("lot_held", { reason: "engineer review" }));
  applyEvent(
    lots,
    tools,
    event("track_out", {
      step_no: 1,
      equipment_id: "CLN-01",
      wafers: ["LOT-0001-W01", "LOT-0001-W02"],
    }),
  );
  expect(lots[0].state).toBe("on_hold");
  expect(lots[0].hold_reason).toBe("engineer review");

  applyEvent(lots, tools, event("lot_queued", { step_no: 2 }));
  expect(lots[0].state).toBe("on_hold");
  expect(lots[0].current_step).toBe(2);

  applyEvent(lots, tools, event("lot_released", {}));
  expect(lots[0].state).toBe("queued");
  expect(lots[0].hold_reason).toBe(null);
  expect(lots[0].wafers.every((wafer) => wafer.state === "queued")).toBe(true);
});

test("completion marks every live wafer done", () => {
  const lots = [lot()];
  const tools = [tool()];
  applyEvent(lots, tools, event("lot_completed", { step_no: 4 }));
  expect(lots[0].state).toBe("completed");
  expect(lots[0].done_wafers).toBe(2);
  expect(lots[0].wafers.every((wafer) => wafer.state === "done")).toBe(true);
});

test("an unknown lot asks for a full snapshot", () => {
  const ok = applyEvent([], [tool()], event("lot_completed", {}));
  expect(ok).toBe(false);
});

test("the clock is UTC time of day", () => {
  expect(clockText(0)).toBe("00:00:00 UTC");
  expect(clockText(3_723_000)).toBe("01:02:03 UTC");
});

test("a live clock advances from the snapshot at the sim speed", () => {
  expect(liveNow(1_000, 10_000, 10, 10_500)).toBe(6_000);
  expect(liveNow(1_000, 10_000, 10, 9_000)).toBe(1_000);
});

test("the first sight of a lot is not a step change", () => {
  const noted = noteSteps({}, [], [lot()], 5_000);
  expect(noted.changed).toBe(false);
  expect(Object.keys(noted.steps).length).toBe(0);
});

test("a real step change records when the lot entered it", () => {
  const before = lot();
  const after = { ...lot(), current_step: 2 };
  const noted = noteSteps({}, [before], [after], 7_000);
  expect(noted.changed).toBe(true);
  expect(noted.steps["LOT-0001"]).toEqual([2, 7_000]);
});
