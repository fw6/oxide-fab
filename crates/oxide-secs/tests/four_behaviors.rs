//! The four required SECS/GEM behaviours, over a real HSMS-SS/TCP session.

#![cfg(feature = "secs-gem")]

use std::time::{Duration, Instant};

use oxide_core::model::Recipe;
use oxide_secs::{start_pair, SecsConfig, SecsHandle, SecsStatus};

fn wait_for(
    handle: &SecsHandle,
    what: &str,
    timeout: Duration,
    predicate: impl Fn(&SecsStatus) -> bool,
) -> SecsStatus {
    let deadline = Instant::now() + timeout;
    loop {
        let status = handle.status();
        if predicate(&status) {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {status:#?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn the_four_gem_behaviours_run_on_a_real_hsms_link() {
    let handle = start_pair(SecsConfig::ephemeral());

    // 1. S1F1: the host probes the equipment, S1F2 answers online.
    let status = wait_for(&handle, "S1F1", Duration::from_secs(10), |s| s.s1f1_ok >= 1);
    assert!(
        status.equipment_online,
        "equipment reports online after S1F17"
    );

    // 2. S6F11: a collection event reaches the host and is acknowledged.
    handle.process_started("LOT-0001", 1, "CLN-01");
    wait_for(&handle, "S6F11", Duration::from_secs(10), |s| {
        s.s6f11_sent >= 1
    });

    // 3. S5F1: an alarm reaches the host and is acknowledged.
    handle.alarm("ETC-01", oxide_secs::ALID_CHAMBER_FAULT, "chamber fault");
    wait_for(&handle, "S5F1", Duration::from_secs(10), |s| {
        s.s5f1_sent >= 1
    });

    // 4. S7: the recipe is downloaded with S7F3, read back with S7F5/S7F6,
    //    and its checksum matches the one the host computed before sending.
    let body = "CLEAN;SC-1;60C;300s";
    handle.upload_recipe("RCP-CLEAN", &Recipe::checksum_of(body), body);
    let status = wait_for(&handle, "S7F6 checksum", Duration::from_secs(10), |s| {
        s.s7f6_verified >= 1
    });
    assert_eq!(status.s7f4_ack, 1, "S7F4 acknowledged the download");
    assert!(
        status.last_error.as_deref() != Some("checksum mismatch"),
        "{status:#?}"
    );

    // 5. Link loss + spooling: events reported while the link is down are
    //    buffered by the equipment and replayed after the reconnect.
    handle.drop_link();
    wait_for(&handle, "link drop", Duration::from_secs(5), |s| {
        s.link_drops >= 1
    });
    // The host is deliberately held off for 1.5 s, so this event is spooled.
    handle.process_completed("LOT-0001", 1, "CLN-01");
    let status = wait_for(&handle, "spooled event", Duration::from_secs(10), |s| {
        s.spool_depth >= 1
    });
    // Everything this run reported went through the spool: two collection
    // events and one alarm.
    assert_eq!(status.spooled_total, 3);
    assert_eq!(status.spool_depth, 1, "the event waits for the link");
    let status = wait_for(&handle, "spool replay", Duration::from_secs(10), |s| {
        s.host_connected && s.spooled_replayed >= 1 && s.spool_depth == 0
    });
    assert_eq!(status.s6f11_sent, 2, "the spooled event was replayed");

    let messages: Vec<&str> = status
        .traffic
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    for expected in [
        "S1F1", "S1F2", "S1F17", "S1F18", "S6F11", "S6F12", "S5F1", "S5F2", "S7F3", "S7F4", "S7F5",
        "S7F6",
    ] {
        assert!(
            messages.contains(&expected),
            "expected {expected} in the conversation: {messages:?}"
        );
    }

    handle.shutdown();
}
