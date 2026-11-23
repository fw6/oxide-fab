// `nsh test` runs these in a headless runner built with the crate: the shell,
// every domain, the control bar and the demo all run for real against a
// stubbed API, and the 3D view is faked so the tests need no GPU.
import { expect, open, test } from "nsh/test";

const SNAPSHOT = `{
  "sim": {
    "now_ms":1763290000000,"running":true,"speed":10.0,"released_lots":1,"total_lots":1,"persisted_events":7,
    "steps": [
      {"step_no":1,"name":"clean","equipment_kind":"clean","queue_len":1,"busy_tools":1,"total_tools":2,"current_lots":["LOT-0001"]},
      {"step_no":2,"name":"litho","equipment_kind":"litho","queue_len":0,"busy_tools":0,"total_tools":2,"current_lots":[]},
      {"step_no":3,"name":"etch","equipment_kind":"etch","queue_len":0,"busy_tools":0,"total_tools":1,"current_lots":[]},
      {"step_no":4,"name":"metrology","equipment_kind":"metrology","queue_len":0,"busy_tools":0,"total_tools":1,"current_lots":[]}
    ],
    "equipment": [
      {"id":"CLN-01","kind":"clean","chamber":"C1","state":"running","light":"green","current_lot":"LOT-0001","current_step":1,"lots_processed":3},
      {"id":"CLN-02","kind":"clean","chamber":"C2","state":"down","light":"red","current_lot":null,"current_step":null,"lots_processed":0}
    ],
    "lots": [
      {"id":"LOT-0001","product_id":"PRD-OXIDE-300","state":"queued","current_step":1,"priority":5,
       "wafer_count":2,"done_wafers":0,"hold_reason":null,"parent_lot_id":null,
       "wafers":[
         {"id":"LOT-0001-W01","slot":1,"state":"queued","defect_info":null,"rework_count":0},
         {"id":"LOT-0001-W02","slot":2,"state":"queued","defect_info":null,"rework_count":0}
       ]}
    ],
    "dispatch": [
      {"at_ms":1763290000000,"lot_id":"LOT-0001","step_no":1,"equipment_id":"CLN-01","priority":5,
       "queue_position":0,"queue_len":1,"qualified_tools":["CLN-01","CLN-02"],"recipe_id":"RCP-CLEAN",
       "rule":"优先级 → FIFO"}
    ]
  },
  "db": {"lots":1,"wafers":2,"events":7,"outbox_pending":0,"outbox_published":7,"published_events":7},
  "outbox": {"pending":0,"published":7,"oldest_pending_ms":0,"max_attempts":0},
  "last_seq": 7,
  "greptime_ok": true,
  "secs": {
    "host_connected":true,"equipment_online":true,"s1f1_ok":1,"s6f11_sent":2,"s5f1_sent":1,
    "s7f3_uploaded":1,"s7f4_ack":1,"s7f6_verified":1,"spool_depth":0,"spooled_total":3,
    "spooled_replayed":3,"link_drops":0,"last_error":null,"traffic":[]
  },
  "backup": {"taken_at":"20261116T120000Z","pg_full_backup":"20261116-120000F","wal_continuity":"2/0/000000010000000000000016","ok":true},
  "restore": {"drilled_at":"20261116T121000Z","target_time":"2026-11-16 12:09:00+00","lots":6,"events":102,"greptime_rows":24,"elapsed_ms":91000,"ok":true},
  "spc": [
    {"ts":"2026-11-16T12:00:00Z","equipment_id":"MET-01","param":"metrology_cd_nm","avg_value":100.2,"min_value":100.0,"max_value":100.4,"ucl":103.0,"lcl":97.0}
  ],
  "spc_source": "flow",
  "chaos": [{"at_ms":1,"pending":0,"published":7,"oldest_pending_ms":0,"delivered_rows":10}]
}`;

const DETAIL = `{
  "lot": {"id":"LOT-0001","product_id":"PRD-OXIDE-300","state":"queued","current_step":1,"priority":5,
          "wafer_count":2,"done_wafers":0,"hold_reason":null,"parent_lot_id":null,"wafers":[]},
  "wafers": [
    {"id":"LOT-0001-W01","slot":1,"state":"queued","defect_info":null,"rework_count":0},
    {"id":"LOT-0001-W02","slot":2,"state":"queued","defect_info":null,"rework_count":0}
  ],
  "genealogy": [
    {"wafer_id":"LOT-0001-W01","lot_id":"LOT-0001","slot":1,"state":"queued","defect_info":null,"rework_count":0,
     "steps":[{"step_no":1,"equipment_id":"CLN-01","recipe_id":"RCP-CLEAN","track_in_ms":10,"track_out_ms":6010,"duration_ms":6000,"measured":null}]}
  ],
  "events": []
}`;

const ANALYTICS = `{
  "recipes": [
    {"recipe_id":"RCP-CLEAN","version":3,"checksum":"abc123def456","steps":[1],"runs":1,
     "verified_runs":1,"mismatched_runs":0,"last_run_ms":1763290000000,"status":"verified"}
  ],
  "qtime_limits": [
    {"from_step":1,"to_step":2,"limit_ms":60000},
    {"from_step":2,"to_step":3,"limit_ms":60000},
    {"from_step":3,"to_step":4,"limit_ms":60000}
  ],
  "qtime_violations": [
    {"lot_id":"LOT-0001","wafer_ids":["LOT-0001-W01"],"from_step":1,"to_step":2,"elapsed_ms":90000,
     "limit_ms":60000,"occurred_at_ms":1763290000000,
     "event_id":"11111111-1111-1111-1111-111111111111","equipment_id":"LIT-01"}
  ],
  "alarms": [
    {"event_id":"22222222-2222-2222-2222-222222222222","equipment_id":"CLN-02","kind":"down",
     "at_ms":1763290000000,"reason":"vacuum lost","down_ms":0}
  ],
  "kpi": {
    "lots_created":1,"lots_completed":1,"wafers_created":2,"wafers_scrapped":0,
    "first_pass_yield":1.0,"mean_cycle_time_ms":60000.0,"max_cycle_time_ms":60000,
    "throughput_lots_per_hour":60.0,"wip_lots":0,
    "window_start_ms":1763290000000,"window_end_ms":1763290060000,
    "tools": [
      {"equipment_id":"CLN-01","kind":"clean","busy_ms":6000,"down_ms":0,"track_ins":1,
       "track_outs":1,"utilization":0.1,"availability":1.0}
    ],
    "defects": [],
    "step_cycles": [
      {"step_no":1,"planned_ms":6000,"actual_mean_ms":6000.0,"samples":1}
    ],
    "lot_cycles": [
      {"lot_id":"LOT-0001","created_ms":1763290000000,"completed_ms":1763290060000,"cycle_ms":60000}
    ]
  },
  "notes": [
    {"metric":"周期时间","source":"lot_event","formula":"首事件到 lot_completed 的差"}
  ]
}`;

const MONITOR = `{"components":[
  {"name":"postgres","status":"up","detail":"PostgreSQL answers SELECT 1","latency_ms":1},
  {"name":"greptimedb","status":"up","detail":"GreptimeDB answers SELECT 1","latency_ms":1},
  {"name":"object-store","status":"up","detail":"minio answers","latency_ms":1},
  {"name":"outbox-publisher","status":"up","detail":"outbox empty","latency_ms":1},
  {"name":"oxide-api","status":"up","detail":"this API","latency_ms":1}
],"checked_at_ms":1763290000000}`;

/** The 3D pane and the desktop actions, without a GPU or a save dialog.
 *
 * Stand-ins replace exports and their members one by one (a nested object
 * would be handed over as plain JSON), so each member is named with a dot.
 */
const MODULES = {
  "oxide-ui/native": {
    "Viewport.sink": () => 0,
    "Viewport.setScene": (_scene: unknown) => undefined,
    "Viewport.setSize": (_width: number, _height: number) => undefined,
    "Viewport.tick": () => 0,
    "Viewport.adapter": () => "",
    "Viewport.error": () => null,
    "Viewport.orbit": (_dx: number, _dy: number) => undefined,
    "Viewport.pan": (_dx: number, _dy: number) => undefined,
    "Viewport.zoom": (_notches: number) => undefined,
    "Viewport.toggleProjection": () => "perspective",
    "Viewport.resetCamera": () => undefined,
    "Viewport.focus": (_entity: unknown) => undefined,
    "Viewport.pick": (_x: number, _y: number) => null,
    "Viewport.describe": () => "{}",
    "Desktop.saveJson": (_name: string, _contents: string) => "export.json",
    "Desktop.startBackend": (_root: string) => "后端已就绪",
  },
};

function routes(snapshot: string) {
  return {
    "GET http://127.0.0.1:18080/api/snapshot": { body: snapshot },
    "GET http://127.0.0.1:18080/api/events*": { body: { events: [], last_seq: 7 } },
    "GET http://127.0.0.1:18080/api/lots/LOT-0001": { body: DETAIL },
    "GET http://127.0.0.1:18080/api/analytics": { body: ANALYTICS },
    "GET http://127.0.0.1:18080/api/monitor": { body: MONITOR },
  };
}

async function dashboard(snapshot = SNAPSHOT) {
  return open("./App.tsx", {
    width: 1280,
    height: 1600,
    network: routes(snapshot),
    modules: MODULES,
  });
}

async function postedUrls(app: Awaited<ReturnType<typeof open>>, want: number): Promise<string[]> {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const posted = app
      .requests()
      .filter((request) => request.method === "POST")
      .map((request) => request.url);
    if (posted.length >= want) return posted;
    await app.wait(50);
  }
  return app
    .requests()
    .filter((request) => request.method === "POST")
    .map((request) => request.url);
}

test("the workbench renders its shell and the MES domain", async () => {
  const app = await dashboard();
  await app.waitFor("text:氧化层 · oxide-fab");
  expect(app.texts()).toContain("3D 工作台");
  await app.waitFor("text:工序队列 · WIP");
  await app.waitFor("text:机台三态 · E116 时长");
  await app.waitFor("text:批次与逐片追溯");
  await app.waitFor("text:LOT-0001");
  await app.waitFor("text:CLN-01");
  await app.waitFor("text:L0 全厂 · 全厂");
  expect(app.texts().some((text) => text.includes("排队"))).toBe(true);
});

test("pressing a lot expands its wafers and genealogy", async () => {
  const app = await dashboard();
  await app.waitFor("text:LOT-0001");
  await app.press("text:LOT-0001 >> nth=1");
  await app.waitFor("text:W01");
  await app.waitFor("text:W02");
  await app.press("text:W01");
  await app.waitFor("text:RCP-CLEAN");
  await app.waitFor("text:CLN-01");
});

test("the quality domain shows SPC, alarms, Q-time and recipes", async () => {
  const app = await dashboard();
  await app.waitFor("text:LOT-0001");
  await app.press("text:工艺质量");
  await app.waitFor("text:SPC · 控制图");
  await app.waitFor("text:告警与 OCAP 路径");
  await app.waitFor("text:Q-time · 连续工序上限");
  await app.waitFor("text:配方 · RMS 校验");
  await app.waitFor("text:RCP-CLEAN v3 · abc123def4");
});

test("the cross-cutting domain shows the monitor and the outbox", async () => {
  const app = await dashboard();
  await app.waitFor("text:LOT-0001");
  await app.press("text:横切 · 总线与 KPI");
  await app.waitFor("text:组件巡查 · SmartFactory Monitor");
  await app.waitFor("text:消息总线 · outbox 积压与追平");
  await app.waitFor("text:KPI · 口径化指标");
  await app.waitFor("text:postgres");
});

test("the control bar pauses, steps and changes the speed", async () => {
  const app = await dashboard();
  await app.waitFor("text:LOT-0001");
  await app.press("text:暂停");
  await app.press("text:单步");
  await app.press("text:− >> nth=0");
  await app.press("text:+ >> nth=0");
  const posted = await postedUrls(app, 4);
  for (const expected of ["/api/sim/pause", "/api/sim/step", "/api/sim/speed/5", "/api/sim/speed/20"]) {
    expect(posted.some((url) => url.endsWith(expected))).toBe(true);
  }
});

test("the same button plays a paused line", async () => {
  const app = await dashboard(SNAPSHOT.replace('"running":true', '"running":false'));
  await app.waitFor("text:LOT-0001");
  await app.press("text:运行");
  const posted = await postedUrls(app, 1);
  expect(posted.some((url) => url.endsWith("/api/sim/play"))).toBe(true);
});

test("the demo starts and stops without jumping back", async () => {
  const app = await dashboard();
  await app.waitFor("text:LOT-0001");
  await app.press("text:演示");
  await app.waitFor("text:1/8 全厂 · MES");
  await app.press("text:停止演示");
  await app.waitFor("text:演示待启动");
});

test("the workbench survives an unreachable API", async () => {
  const app = await open("./App.tsx", { width: 1280, height: 1600, network: {}, modules: MODULES });
  await app.waitFor("text:氧化层 · oxide-fab");
  await app.waitFor("text:重连中");
});
