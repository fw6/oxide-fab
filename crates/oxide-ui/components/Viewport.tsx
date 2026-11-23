/** The 3D pane: a native view fed by the Rust renderer, with pointer input.
 *
 * TSX sends the scene and the view size, ticks the renderer about 20 times a
 * second, and turns drags, wheel and clicks into camera calls and picks.
 */
import { cleanup, effect, nativeComponent, signal, View } from "nsh";
import {
  adapter as adapterText,
  drill,
  dark,
  explosion,
  focus,
  greptimeOk,
  level,
  lots,
  nowMs,
  outboxPending,
  reduceMotion,
  renderError,
  running,
  select,
  selected,
  speed,
  stepSince,
  steps,
  tools,
  viewport,
} from "../store";
import { fromEntityInput, toEntityInput } from "../model";
import { p } from "./ui";

interface NativeWheel {
  x: number;
  y: number;
  dx: number;
  dy: number;
  notches_y: number;
  precise: boolean;
  shift: boolean;
  ctrl: boolean;
}

const View3D = nativeComponent<{ sink: number; frame: number }, { nativewheel: NativeWheel }>("oxide.view3d", {
  props: ["sink", "frame"],
  events: ["nativewheel"],
});

function sceneInput() {
  const since = stepSince();
  return {
    nowMs: nowMs(),
    running: running(),
    speed: speed(),
    steps: steps().map((step) => ({
      stepNo: step.step_no,
      name: step.name,
      equipmentKind: step.equipment_kind,
      queueLen: step.queue_len,
      busyTools: step.busy_tools,
      totalTools: step.total_tools,
      currentLots: step.current_lots,
    })),
    tools: tools().map((tool) => ({
      id: tool.id,
      kind: tool.kind,
      state: tool.state,
      light: tool.light,
      currentLot: tool.current_lot,
      currentStep: tool.current_step,
      lotsProcessed: tool.lots_processed,
    })),
    lots: lots().map((lot) => ({
      id: lot.id,
      state: lot.state,
      currentStep: lot.current_step,
      priority: lot.priority,
      waferCount: lot.wafer_count,
      doneWafers: lot.done_wafers,
      holdReason: lot.hold_reason,
      stepSinceMs: since[lot.id]?.[0] === lot.current_step ? since[lot.id][1] : 0,
      wafers: lot.wafers.map((wafer) => ({
        id: wafer.id,
        slot: wafer.slot,
        state: wafer.state,
        defectInfo: wafer.defect_info,
        reworkCount: wafer.rework_count,
      })),
    })),
    level: level(),
    focus: toEntityInput(focus()),
    selected: toEntityInput(selected()),
    explosion: explosion(),
    reduceMotion: reduceMotion(),
    alert: !greptimeOk() || outboxPending() > 0,
    dark: dark(),
  };
}

/** The render loop's cadence: a frame every 50 ms while the view is on. */
const TICK_MS = 50;

export function ViewportPane() {
  const frame = signal(0);
  const width = signal(0);
  const height = signal(0);

  let drag = { active: false, lastX: 0, lastY: 0, moved: 0 };

  effect(() => {
    viewport.setScene(sceneInput());
  });
  effect(() => {
    viewport.setSize(width(), height());
  });

  const ticker = setInterval(() => {
    frame.set(viewport.tick());
    const nextAdapter = viewport.adapter();
    if (nextAdapter !== "" && nextAdapter !== adapterText()) adapterText.set(nextAdapter);
    const nextError = viewport.error();
    if (nextError !== renderError()) renderError.set(nextError);
  }, TICK_MS);
  cleanup(() => clearInterval(ticker));

  return (
    <View
      style={{
        flex: 1,
        minHeight: 0,
        borderRadius: 10,
        overflow: "hidden",
        borderWidth: 1,
        borderColor: p().border,
        backgroundColor: p().scene,
      }}
    >
      <View3D
        sink={viewport.sink()}
        frame={frame()}
        style={{ flex: 1 }}
        onLayout={(event) => {
          const size = event.bounds.size;
          if (size.width > 0 && size.height > 0) {
            width.set(size.width);
            height.set(size.height);
          }
        }}
        onMouseMove={(event) => {
          const x = event.position.x;
          const y = event.position.y;
          if (event.pressedButton !== null) {
            if (drag.active) {
              const dx = x - drag.lastX;
              const dy = y - drag.lastY;
              drag.moved += Math.hypot(dx, dy);
              drag.lastX = x;
              drag.lastY = y;
              if (dx !== 0 || dy !== 0) {
                if (event.modifiers.shift) viewport.pan(dx, dy);
                else viewport.orbit(dx, dy);
              }
            } else {
              drag = { active: true, lastX: x, lastY: y, moved: 0 };
            }
          } else {
            drag.active = false;
          }
        }}
        onMouseUp={(event) => {
          const wasClick = drag.active && drag.moved < 5 && event.button === "left";
          drag.active = false;
          if (wasClick) {
            const hit = viewport.pick(event.position.x, event.position.y);
            const entity = fromEntityInput(hit);
            if (entity === null) select(null);
            else drill(entity);
          }
        }}
        onWheel={(event) => {
          const delta = event.deltaMode === "lines" ? event.delta.y * 16 : event.delta.y;
          viewport.zoom(-delta);
        }}
        onNativewheel={(event) => {
          viewport.zoom(-(event.dy / 40) - event.notches_y * 1.5);
        }}
      />
    </View>
  );
}
