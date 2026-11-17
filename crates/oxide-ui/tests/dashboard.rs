//! Headless UI tests: the workbench mounts in the real engine, renders the
//! shell and every domain from a stubbed API, and drives the same endpoints
//! the scripts use.
//!
//! These tests run in `cargo test` and therefore in `nsh test`. The 3D viewport
//! is disabled here (`OXIDE_RENDER_DISABLE=1`): its wgpu path has its own
//! headless evidence binary, and a UI test must not need a GPU.

use std::sync::{Arc, OnceLock};

use nsh_core::geometry::{Size, px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::{Headless, StubNetwork, StubResponse};

const SNAPSHOT: &str = r#"{
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
}"#;

const EVENTS: &str = r#"{"events":[],"last_seq":7}"#;

const DETAIL: &str = r#"{
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
}"#;

const ANALYTICS: &str = r#"{
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
}"#;

const MONITOR: &str = r#"{"components":[
  {"name":"postgres","status":"up","detail":"PostgreSQL answers SELECT 1","latency_ms":1},
  {"name":"greptimedb","status":"up","detail":"GreptimeDB answers SELECT 1","latency_ms":1},
  {"name":"object-store","status":"up","detail":"minio answers","latency_ms":1},
  {"name":"outbox-publisher","status":"up","detail":"outbox empty","latency_ms":1},
  {"name":"oxide-api","status":"up","detail":"this API","latency_ms":1}
],"checked_at_ms":1763290000000}"#;

fn json(body: &str) -> StubResponse {
    let mut response = StubResponse::default();
    response.status = 200;
    response.headers = vec![("Content-Type".to_owned(), "application/json".to_owned())];
    response.body = body.as_bytes().to_vec();
    response
}

/// One process, one GPU policy: the UI tests never open the renderer.
fn disable_render() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        // SAFETY: tests set the same value once, before any app starts; no
        // other thread reads the environment concurrently in this process.
        unsafe { std::env::set_var("OXIDE_RENDER_DISABLE", "1") };
    });
}

fn dashboard_with_snapshot(snapshot: &str) -> (Headless, Arc<StubNetwork>) {
    disable_render();
    let network = Arc::new(StubNetwork::new([
        (
            "GET http://127.0.0.1:8080/api/snapshot".to_owned(),
            json(snapshot),
        ),
        (
            "GET http://127.0.0.1:8080/api/events*".to_owned(),
            json(EVENTS),
        ),
        (
            "GET http://127.0.0.1:8080/api/lots/LOT-0001".to_owned(),
            json(DETAIL),
        ),
        (
            "GET http://127.0.0.1:8080/api/analytics".to_owned(),
            json(ANALYTICS),
        ),
        (
            "GET http://127.0.0.1:8080/api/monitor".to_owned(),
            json(MONITOR),
        ),
    ]));
    let mut config = EngineConfig::new(Root::of::<oxide_ui::App>());
    // A desktop window, so every panel is laid out and on screen.
    config.viewport = Size::new(px(1280.0), px(1600.0));
    config.network = Some(network.clone());
    let app = Headless::start(config).expect("the workbench starts");
    (app, network)
}

fn dashboard() -> Headless {
    dashboard_with_snapshot(SNAPSHOT).0
}

#[test]
fn the_workbench_renders_its_shell_and_the_mes_domain() {
    let mut app = dashboard();
    app.wait_for("text:氧化层 · oxide-fab").unwrap();
    app.wait_for("text:3D 工作台").unwrap();
    app.wait_for("text:工序队列 · WIP").unwrap();
    app.wait_for("text:机台三态 · E116 时长").unwrap();
    app.wait_for("text:批次与逐片追溯").unwrap();
    app.wait_for("text:LOT-0001").unwrap();
    app.wait_for("text:CLN-01").unwrap();
    app.wait_for("text:L0 全厂 · 全厂").unwrap();
    let texts = app.screen().texts();
    assert!(
        texts.iter().any(|text| text.contains("排队")),
        "the queue counters are on screen: {texts:?}"
    );
}

#[test]
fn pressing_a_lot_expands_its_wafers_and_genealogy() {
    let mut app = dashboard();
    app.wait_for("text:LOT-0001").unwrap();
    app.press("text:LOT-0001 >> nth=1").unwrap();
    app.wait_for("text:W01").unwrap();
    app.wait_for("text:W02").unwrap();
    app.press("text:W01").unwrap();
    app.wait_for("text:RCP-CLEAN").unwrap();
    app.wait_for("text:CLN-01").unwrap();
}

#[test]
fn the_quality_domain_shows_spc_alarms_qtime_and_recipes() {
    let mut app = dashboard();
    app.wait_for("text:LOT-0001").unwrap();
    app.press("text:工艺质量").unwrap();
    app.wait_for("text:SPC · 控制图").unwrap();
    app.wait_for("text:告警与 OCAP 路径").unwrap();
    app.wait_for("text:Q-time · 连续工序上限").unwrap();
    app.wait_for("text:配方 · RMS 校验").unwrap();
    app.wait_for("text:RCP-CLEAN v3 · abc123def4").unwrap();
}

#[test]
fn the_cross_cutting_domain_shows_the_monitor_and_the_outbox() {
    let mut app = dashboard();
    app.wait_for("text:LOT-0001").unwrap();
    app.press("text:横切 · 总线与 KPI").unwrap();
    app.wait_for("text:组件巡查 · SmartFactory Monitor")
        .unwrap();
    app.wait_for("text:消息总线 · outbox 积压与追平").unwrap();
    app.wait_for("text:KPI · 口径化指标").unwrap();
    app.wait_for("text:postgres").unwrap();
}

/// The control bar is the UI half of §8: the buttons POST to the same
/// `/api/sim/*` endpoints the scripts use.
fn posted_urls(app: &mut Headless, network: &StubNetwork, want: usize) -> Vec<String> {
    use std::time::Duration;

    let mut posted: Vec<String> = Vec::new();
    for _ in 0..60 {
        posted = network
            .requests()
            .iter()
            .filter(|request| request.method == "POST")
            .map(|request| request.url.clone())
            .collect();
        if posted.len() >= want {
            break;
        }
        app.wait(Duration::from_millis(50)).unwrap();
    }
    posted
}

#[test]
fn the_control_bar_pauses_steps_and_changes_the_speed() {
    let (mut app, network) = dashboard_with_snapshot(SNAPSHOT);
    app.wait_for("text:LOT-0001").unwrap();

    app.press("text:暂停").unwrap();
    app.press("text:单步").unwrap();
    app.press("text:− >> nth=0").unwrap(); // ×10 -> 5
    app.press("text:+ >> nth=0").unwrap(); // ×10 -> 20

    let posted = posted_urls(&mut app, &network, 4);
    for expected in [
        "/api/sim/pause",
        "/api/sim/step",
        "/api/sim/speed/5",
        "/api/sim/speed/20",
    ] {
        assert!(
            posted.iter().any(|url| url.ends_with(expected)),
            "{expected} was not posted; the app sent {posted:?}"
        );
    }
}

#[test]
fn the_same_button_plays_a_paused_line() {
    let paused = SNAPSHOT.replace("\"running\":true", "\"running\":false");
    let (mut app, network) = dashboard_with_snapshot(&paused);
    app.wait_for("text:LOT-0001").unwrap();

    app.press("text:运行").unwrap();

    let posted = posted_urls(&mut app, &network, 1);
    assert!(
        posted.iter().any(|url| url.ends_with("/api/sim/play")),
        "/api/sim/play was not posted; the app sent {posted:?}"
    );
}

#[test]
fn the_demo_starts_and_stops_without_jumping_back() {
    let mut app = dashboard();
    app.wait_for("text:LOT-0001").unwrap();
    app.press("text:演示").unwrap();
    app.wait_for("text:1/8 全厂 · MES").unwrap();
    app.press("text:停止演示").unwrap();
    app.wait_for("text:演示待启动").unwrap();
}

#[test]
fn the_workbench_survives_an_unreachable_api() {
    disable_render();
    let mut config = EngineConfig::new(Root::of::<oxide_ui::App>());
    config.network = Some(Arc::new(StubNetwork::new(
        Vec::<(String, StubResponse)>::new(),
    )));
    let mut app = Headless::start(config).expect("the workbench starts offline");
    app.wait_for("text:氧化层 · oxide-fab").unwrap();
    app.wait_for("text:重连中").unwrap();
}
