//! Headless UI tests: the dashboard mounts in the real engine, renders the
//! floor from a stubbed API, and expands a lot into its wafers.
//!
//! These tests run in `cargo test` and therefore in `nsh test`.

use std::sync::Arc;

use nsh_core::geometry::{px, Size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::{Headless, StubNetwork, StubResponse};

const SNAPSHOT: &str = r#"{
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
  "chaos": [{"at_ms":1,"pending":0,"published":7,"oldest_pending_ms":0,"delivered_rows":10}],
  "sim": {"now_ms":1763290000000,"running":true,"speed":10.0,"released_lots":1,"total_lots":1,"persisted_events":7}
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

fn json(body: &str) -> StubResponse {
    let mut response = StubResponse::default();
    response.status = 200;
    response.headers = vec![("Content-Type".to_owned(), "application/json".to_owned())];
    response.body = body.as_bytes().to_vec();
    response
}

fn dashboard() -> Headless {
    let network = StubNetwork::new([
        (
            "GET http://127.0.0.1:8080/api/snapshot".to_owned(),
            json(SNAPSHOT),
        ),
        (
            "GET http://127.0.0.1:8080/api/events*".to_owned(),
            json(EVENTS),
        ),
        (
            "GET http://127.0.0.1:8080/api/lots/LOT-0001".to_owned(),
            json(DETAIL),
        ),
    ]);
    let mut config = EngineConfig::new(Root::of::<oxide_ui::App>());
    // A desktop window, so every panel is laid out and on screen.
    config.viewport = Size::new(px(1280.0), px(1600.0));
    config.network = Some(Arc::new(network));
    Headless::start(config).expect("the dashboard starts")
}

#[test]
fn the_dashboard_renders_queues_tools_and_the_lot_list() {
    let mut app = dashboard();
    app.wait_for("text:氧化层 · oxide-fab").unwrap();
    app.wait_for("text:四条工序队列").unwrap();
    app.wait_for("text:机台灯号").unwrap();
    app.wait_for("text:LOT-0001").unwrap();
    app.wait_for("text:CLN-01").unwrap();
    app.wait_for("text:1. clean").unwrap();
    app.wait_for("text:SECS/GEM 假机台").unwrap();
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
    app.press("text:LOT-0001").unwrap();
    app.wait_for("text:W01").unwrap();
    app.wait_for("text:W02").unwrap();
    app.press("text:W01").unwrap();
    app.wait_for("text:RCP-CLEAN").unwrap();
    app.wait_for("text:CLN-01").unwrap();
}

#[test]
fn the_dashboard_survives_an_unreachable_api() {
    // No routes: every fetch fails, so the shell must still render and say it
    // is reconnecting instead of panicking.
    let mut config = EngineConfig::new(Root::of::<oxide_ui::App>());
    config.network = Some(Arc::new(StubNetwork::new(Vec::<(String, StubResponse)>::new())));
    let mut app = Headless::start(config).expect("the dashboard starts offline");
    app.wait_for("text:氧化层 · oxide-fab").unwrap();
    app.wait_for("text:重连中").unwrap();
}
