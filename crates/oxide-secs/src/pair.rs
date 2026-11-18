//! The simulated equipment / host pair.

use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use secs4rs::gem::{
    Ackc5, Ackc6, Ackc7, CommAck, GemConfig, OnlAck, s1f1, s1f2, s1f13, s1f14, s1f17, s1f18,
    s5f1_alarm, s5f2, s6f11_event, s6f11_report, s6f12, s7f4,
};
use secs4rs::hsms::HsmsMessage;
use secs4rs::hsms_ss::{HsmsSsCommunicator, HsmsSsCommunicatorConfig};
use secs4rs::secs2::Secs2;
use serde::{Deserialize, Serialize};

/// Where the pair listens and which SECS session it uses.
#[derive(Debug, Clone)]
pub struct SecsConfig {
    pub bind: SocketAddr,
    pub session_id: i32,
    pub mdln: String,
    pub softrev: String,
}

impl Default for SecsConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:50010".parse().expect("valid default address"),
            session_id: 10,
            mdln: "OXIDE-SIM-EQUIPMENT".to_owned(),
            softrev: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

impl SecsConfig {
    /// A config on a free loopback port, for tests and for running two pairs
    /// side by side.
    #[must_use]
    pub fn ephemeral() -> Self {
        let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
        let addr = probe.local_addr().expect("probe address");
        drop(probe);
        Self {
            bind: addr,
            ..Self::default()
        }
    }
}

/// One line of the SECS conversation, for the dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrafficEntry {
    pub at_ms: i64,
    pub from: String,
    pub message: String,
    pub detail: String,
}

/// What the dashboard shows about the protocol layer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecsStatus {
    pub host_connected: bool,
    pub equipment_online: bool,
    pub session_id: i32,
    pub mdln: String,
    pub softrev: String,
    /// S1F1 sent by the host and answered with S1F2.
    pub s1f1_ok: u64,
    /// S6F11 collection events reported by the equipment.
    pub s6f11_sent: u64,
    /// S5F1 alarm reports reported by the equipment.
    pub s5f1_sent: u64,
    /// S7F3 program downloads acknowledged with S7F4.
    pub s7f3_uploaded: u64,
    pub s7f4_ack: u64,
    /// S7F5/S7F6 read-backs whose checksum matched the host's own.
    pub s7f6_verified: u64,
    /// Events waiting in the equipment spool because the link is down.
    pub spool_depth: u64,
    /// Events spooled in total (including ones already replayed).
    pub spooled_total: u64,
    /// Spooled events replayed after a reconnect.
    pub spooled_replayed: u64,
    pub link_drops: u64,
    pub last_error: Option<String>,
    pub traffic: Vec<TrafficEntry>,
}

#[derive(Debug, Clone)]
enum EquipmentTask {
    ProcessStarted {
        lot_id: String,
        step_no: i32,
        equipment_id: String,
    },
    ProcessCompleted {
        lot_id: String,
        step_no: i32,
        equipment_id: String,
    },
    Alarm {
        equipment_id: String,
        alid: u32,
        text: String,
    },
}

#[derive(Debug, Clone)]
enum HostTask {
    UploadRecipe {
        recipe_id: String,
        checksum: String,
        body: String,
    },
    /// Drop the session and stay away for a while, so spooling is
    /// deterministic instead of racing the reconnect.
    DropLink { hold_ms: i64 },
}

/// A handle on the running pair.
pub struct SecsHandle {
    status: Arc<Mutex<SecsStatus>>,
    /// The GEM spool: collection events and alarms wait here until the link
    /// is up, exactly like an equipment's S2F43 spool.
    spool: Spool,
    host_tasks: Sender<HostTask>,
    shutdown: Arc<AtomicBool>,
}

impl std::fmt::Debug for SecsHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecsHandle").finish_non_exhaustive()
    }
}

impl SecsHandle {
    #[must_use]
    pub fn status(&self) -> SecsStatus {
        self.status.lock().expect("status lock").clone()
    }

    pub fn process_started(&self, lot_id: &str, step_no: i32, equipment_id: &str) {
        self.push_equipment(EquipmentTask::ProcessStarted {
            lot_id: lot_id.to_owned(),
            step_no,
            equipment_id: equipment_id.to_owned(),
        });
    }

    pub fn process_completed(&self, lot_id: &str, step_no: i32, equipment_id: &str) {
        self.push_equipment(EquipmentTask::ProcessCompleted {
            lot_id: lot_id.to_owned(),
            step_no,
            equipment_id: equipment_id.to_owned(),
        });
    }

    pub fn alarm(&self, equipment_id: &str, alid: u32, text: &str) {
        self.push_equipment(EquipmentTask::Alarm {
            equipment_id: equipment_id.to_owned(),
            alid,
            text: text.to_owned(),
        });
    }

    pub fn upload_recipe(&self, recipe_id: &str, checksum: &str, body: &str) {
        let _ = self.host_tasks.send(HostTask::UploadRecipe {
            recipe_id: recipe_id.to_owned(),
            checksum: checksum.to_owned(),
            body: body.to_owned(),
        });
    }

    /// Close the HSMS session for 1.5 s; the equipment spools what happens
    /// meanwhile and replays it once the host is back.
    pub fn drop_link(&self) {
        self.drop_link_for(1500);
    }

    /// Close the HSMS session for `hold_ms` (100 ms .. 30 s).
    pub fn drop_link_for(&self, hold_ms: i64) {
        let _ = self.host_tasks.send(HostTask::DropLink {
            hold_ms: hold_ms.clamp(100, 30_000),
        });
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    fn push_equipment(&self, task: EquipmentTask) {
        let depth = {
            let mut queue = self.spool.lock().expect("spool");
            queue.push_back(task);
            queue.len() as u64
        };
        let mut status = self.status.lock().expect("status lock");
        status.spooled_total += 1;
        status.spool_depth = depth;
    }
}

type Recipes = Arc<Mutex<HashMap<String, String>>>;
type Spool = Arc<Mutex<VecDeque<EquipmentTask>>>;

/// Start the simulated equipment (passive) and host (active).
#[must_use]
#[expect(
    clippy::needless_pass_by_value,
    reason = "callers own a fresh config; the pair keeps it"
)]
pub fn start_pair(config: SecsConfig) -> SecsHandle {
    let status = Arc::new(Mutex::new(SecsStatus {
        session_id: config.session_id,
        mdln: config.mdln.clone(),
        softrev: config.softrev.clone(),
        ..SecsStatus::default()
    }));
    let spool: Spool = Arc::new(Mutex::new(VecDeque::new()));
    let (host_tx, host_rx) = mpsc::channel();
    let shutdown = Arc::new(AtomicBool::new(false));
    let reconnect_pause_until_ms = Arc::new(AtomicI64::new(0));
    let recipes: Recipes = Arc::new(Mutex::new(HashMap::new()));

    {
        let config = config.clone();
        let status = Arc::clone(&status);
        let shutdown = Arc::clone(&shutdown);
        let recipes = Arc::clone(&recipes);
        let spool = Arc::clone(&spool);
        thread::spawn(move || {
            equipment_loop(config, status, shutdown, recipes, spool);
        });
    }
    {
        let config = config.clone();
        let status = Arc::clone(&status);
        let shutdown = Arc::clone(&shutdown);
        let pause = Arc::clone(&reconnect_pause_until_ms);
        thread::spawn(move || {
            host_loop(config, status, host_rx, shutdown, pause);
        });
    }

    SecsHandle {
        status,
        spool,
        host_tasks: host_tx,
        shutdown,
    }
}

// ----------------------------------------------------------------------
// equipment
// ----------------------------------------------------------------------

#[expect(
    clippy::needless_pass_by_value,
    reason = "ownership moves into the equipment thread"
)]
fn equipment_loop(
    config: SecsConfig,
    status: Arc<Mutex<SecsStatus>>,
    shutdown: Arc<AtomicBool>,
    recipes: Recipes,
    spool: Spool,
) {
    while !shutdown.load(Ordering::SeqCst) {
        let comm = Arc::new(HsmsSsCommunicator::new_instance(passive_config(&config)));
        match comm.open_passive() {
            Ok(()) => {
                {
                    let mut guard = status.lock().expect("status lock");
                    guard.host_connected = true;
                    guard.equipment_online = true;
                }
                record(&status, "eq", "HSMS SELECT", "passive session selected");
                serve_equipment(&comm, &config, &status, &recipes, &spool);
                comm.close();
                let mut guard = status.lock().expect("status lock");
                guard.host_connected = false;
                guard.equipment_online = false;
            }
            Err(error) => {
                set_error(&status, format!("equipment listen: {error}"));
                thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

fn serve_equipment(
    comm: &Arc<HsmsSsCommunicator>,
    config: &SecsConfig,
    status: &Arc<Mutex<SecsStatus>>,
    recipes: &Recipes,
    spool: &Spool,
) {
    loop {
        // Report what was spooled while the link was down (S2F43/S6F23
        // spooling, simulated).
        flush_spool(comm, status, spool);
        match comm.poll_data_message(Duration::from_millis(50)) {
            Ok(Some(message)) => handle_equipment_message(comm, config, status, recipes, &message),
            Ok(None) => {}
            Err(_) => return,
        }
    }
}

fn handle_equipment_message(
    comm: &Arc<HsmsSsCommunicator>,
    config: &SecsConfig,
    status: &Arc<Mutex<SecsStatus>>,
    recipes: &Recipes,
    message: &HsmsMessage,
) {
    let gem = GemConfig::new();
    gem.set_mdln(&config.mdln);
    gem.set_softrev(&config.softrev);
    gem.set_is_equip(true);
    let stream = message.get_stream();
    let function = message.get_function();
    record(
        status,
        "host",
        &format!("S{stream}F{function}"),
        "primary received by the equipment",
    );
    let result: Result<(), String> = match (stream, function) {
        (1, 1) => s1f2(comm, message, &gem)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        (1, 13) => s1f14(comm, message, &gem, CommAck::Ok)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        (1, 17) => {
            let result = s1f18(comm, message, OnlAck::Ok)
                .map(|_| ())
                .map_err(|e| e.to_string());
            if result.is_ok() {
                status.lock().expect("status lock").equipment_online = true;
            }
            result
        }
        (7, 3) => handle_download(comm, status, recipes, message),
        (7, 5) => handle_upload_readback(comm, status, recipes, message),
        _ => Ok(()),
    };
    match &result {
        Ok(()) => {
            let reply = match (stream, function) {
                (1, 1) => Some("S1F2"),
                (1, 13) => Some("S1F14"),
                (1, 17) => Some("S1F18"),
                (7, 3) => Some("S7F4"),
                (7, 5) => Some("S7F6"),
                _ => None,
            };
            if let Some(reply) = reply {
                record(status, "eq", reply, "reply sent by the equipment");
            }
        }
        Err(error) => set_error(status, format!("S{stream}F{function}: {error}")),
    }
}

/// S7F3: store the program, then acknowledge with S7F4.
fn handle_download(
    comm: &Arc<HsmsSsCommunicator>,
    status: &Arc<Mutex<SecsStatus>>,
    recipes: &Recipes,
    message: &HsmsMessage,
) -> Result<(), String> {
    let body = message.secs2();
    let ppid = body
        .get_ascii_at(&[0])
        .map_err(|error| format!("S7F3: missing PPID: {error}"))?;
    let program = body
        .get_ascii_at(&[1])
        .map_err(|error| format!("S7F3: missing PPBODY: {error}"))?;
    let ack = if ppid.is_empty() || program.is_empty() || program.len() > 8192 {
        Ackc7::LengthError
    } else {
        recipes
            .lock()
            .expect("recipes")
            .insert(ppid.clone(), program);
        Ackc7::Accepted
    };
    s7f4(comm, message, ack).map_err(|error| format!("S7F4: {error}"))?;
    record(
        status,
        "eq",
        "S7F4",
        &format!("recipe {ppid} acknowledged with {ack:?}"),
    );
    Ok(())
}

/// S7F5: read the stored program back so the host can verify its checksum.
fn handle_upload_readback(
    comm: &Arc<HsmsSsCommunicator>,
    status: &Arc<Mutex<SecsStatus>>,
    recipes: &Recipes,
    message: &HsmsMessage,
) -> Result<(), String> {
    let ppid = message
        .secs2()
        .get_ascii_at(&[])
        .map_err(|error| format!("S7F5: missing PPID: {error}"))?;
    let program = recipes
        .lock()
        .expect("recipes")
        .get(&ppid)
        .cloned()
        .unwrap_or_default();
    let body = Secs2::list([
        Secs2::ascii(ppid).map_err(|error| format!("S7F6 PPID: {error}"))?,
        Secs2::ascii(program).map_err(|error| format!("S7F6 PPBODY: {error}"))?,
    ])
    .map_err(|error| format!("S7F6 body: {error}"))?;
    comm.send_data_reply(message, 7, 6, false, body)
        .map_err(|error| format!("S7F6: {error}"))?;
    record(status, "eq", "S7F6", "program read back for verification");
    Ok(())
}

fn send_task(
    comm: &Arc<HsmsSsCommunicator>,
    status: &Arc<Mutex<SecsStatus>>,
    task: &EquipmentTask,
) -> bool {
    match task {
        EquipmentTask::ProcessStarted {
            lot_id,
            step_no,
            equipment_id,
        } => send_event(
            comm,
            status,
            crate::ceid::PROCESS_STARTED,
            &format!("process started lot={lot_id} step={step_no} eq={equipment_id}"),
        ),
        EquipmentTask::ProcessCompleted {
            lot_id,
            step_no,
            equipment_id,
        } => send_event(
            comm,
            status,
            crate::ceid::PROCESS_COMPLETED,
            &format!("process completed lot={lot_id} step={step_no} eq={equipment_id}"),
        ),
        EquipmentTask::Alarm {
            equipment_id,
            alid,
            text,
        } => match s5f1_alarm(comm, 0x81, *alid, text) {
            Ok(ack) => {
                record(
                    status,
                    "eq",
                    "S5F1",
                    &format!("alarm {alid} on {equipment_id} ack={ack:?}"),
                );
                status.lock().expect("status lock").s5f1_sent += 1;
                true
            }
            Err(error) => {
                set_error(status, format!("S5F1: {error}"));
                false
            }
        },
    }
}

fn send_event(
    comm: &Arc<HsmsSsCommunicator>,
    status: &Arc<Mutex<SecsStatus>>,
    ceid: i64,
    text: &str,
) -> bool {
    // The text is runtime data (lot ids, alarm text), so an item the SECS
    // encoder refuses must end this message, never panic the equipment thread.
    let item = match Secs2::ascii(text) {
        Ok(item) => item,
        Err(error) => {
            set_error(status, format!("S6F11 text: {error}"));
            return false;
        }
    };
    let report = match s6f11_report(Secs2::uint2([1]).expect("constant rptid"), [item]) {
        Ok(report) => report,
        Err(error) => {
            set_error(status, format!("S6F11 report: {error}"));
            return false;
        }
    };
    let data_id = Secs2::uint4([0]).expect("constant data id");
    let ceid_item = Secs2::uint2([ceid as u16]).expect("ceid");
    match s6f11_event(comm, data_id, ceid_item, [report]) {
        Ok(ack) => {
            record(
                status,
                "eq",
                "S6F11",
                &format!("CEID {ceid}: {text} ack={ack:?}"),
            );
            status.lock().expect("status lock").s6f11_sent += 1;
            true
        }
        Err(error) => {
            set_error(status, format!("S6F11: {error}"));
            false
        }
    }
}

fn flush_spool(comm: &Arc<HsmsSsCommunicator>, status: &Arc<Mutex<SecsStatus>>, spool: &Spool) {
    loop {
        let task = {
            let mut queue = spool.lock().expect("spool");
            queue.pop_front()
        };
        let Some(task) = task else { break };
        if send_task(comm, status, &task) {
            {
                let mut guard = status.lock().expect("status lock");
                guard.spooled_replayed += 1;
            }
            let depth = spool.lock().expect("spool").len() as u64;
            status.lock().expect("status lock").spool_depth = depth;
        } else {
            spool.lock().expect("spool").push_front(task);
            break;
        }
    }
}

// ----------------------------------------------------------------------
// host
// ----------------------------------------------------------------------

#[expect(
    clippy::needless_pass_by_value,
    reason = "ownership moves into the host thread"
)]
fn host_loop(
    config: SecsConfig,
    status: Arc<Mutex<SecsStatus>>,
    tasks: Receiver<HostTask>,
    shutdown: Arc<AtomicBool>,
    pause: Arc<AtomicI64>,
) {
    while !shutdown.load(Ordering::SeqCst) {
        let pause_until = pause.load(Ordering::SeqCst);
        if now_ms() < pause_until {
            let remaining = (pause_until - now_ms()).clamp(10, 2000) as u64;
            thread::sleep(Duration::from_millis(remaining));
            continue;
        }
        let comm = Arc::new(HsmsSsCommunicator::new_instance(active_config(&config)));
        match comm.open_active() {
            Ok(()) => {
                status.lock().expect("status lock").host_connected = true;
                record(&status, "host", "HSMS SELECT", "active session selected");

                // The reply loop is a thread of its own: the host must answer
                // S6F11/S5F1 even while its own S7F3 is in flight, or the two
                // single-threaded peers deadlock on each other's W-bit.
                let stop = Arc::new(AtomicBool::new(false));
                let poller = {
                    let comm = Arc::clone(&comm);
                    let status = Arc::clone(&status);
                    let stop = Arc::clone(&stop);
                    thread::spawn(move || serve_host(&comm, &status, &stop))
                };

                handshake(&comm, &config, &status);
                while !stop.load(Ordering::SeqCst) && !shutdown.load(Ordering::SeqCst) {
                    match tasks.try_recv() {
                        Ok(HostTask::DropLink { hold_ms }) => {
                            pause.store(now_ms() + hold_ms, Ordering::SeqCst);
                            status.lock().expect("status lock").link_drops += 1;
                            record(
                                &status,
                                "host",
                                "SEPARATE",
                                "chaos: link dropped on purpose",
                            );
                            break;
                        }
                        Ok(HostTask::UploadRecipe {
                            recipe_id,
                            checksum,
                            body,
                        }) => upload_recipe(&comm, &status, &recipe_id, &checksum, &body),
                        Err(TryRecvError::Empty) => {
                            thread::sleep(Duration::from_millis(20));
                        }
                        Err(TryRecvError::Disconnected) => break,
                    }
                }
                stop.store(true, Ordering::SeqCst);
                comm.close();
                let _ = poller.join();
                status.lock().expect("status lock").host_connected = false;
            }
            Err(error) => {
                set_error(&status, format!("host connect: {error}"));
                thread::sleep(Duration::from_millis(300));
            }
        }
    }
}

fn handshake(comm: &Arc<HsmsSsCommunicator>, config: &SecsConfig, status: &Arc<Mutex<SecsStatus>>) {
    let gem = GemConfig::new();
    gem.set_mdln(&config.mdln);
    gem.set_softrev(&config.softrev);

    match s1f13(comm, &gem) {
        Ok(ack) => record(status, "host", "S1F13", &format!("commack={ack:?}")),
        Err(error) => set_error(status, format!("S1F13: {error}")),
    }
    match s1f17(comm) {
        Ok(ack) => record(status, "host", "S1F17", &format!("onlack={ack:?}")),
        Err(error) => set_error(status, format!("S1F17: {error}")),
    }
    match s1f1(comm) {
        Ok(Some(reply)) => {
            // The handshake is the point at which the link is known good, so
            // an earlier connection error is no longer worth showing.
            status.lock().expect("status lock").last_error = None;
            let mdln = reply
                .secs2()
                .get_ascii_at(&[0])
                .unwrap_or_else(|_| "<none>".to_owned());
            let softrev = reply
                .secs2()
                .get_ascii_at(&[1])
                .unwrap_or_else(|_| "<none>".to_owned());
            status.lock().expect("status lock").s1f1_ok += 1;
            record(
                status,
                "host",
                "S1F1",
                &format!("S1F2 online mdln={mdln} softrev={softrev}"),
            );
        }
        Ok(None) => set_error(status, "S1F1 got no reply".to_owned()),
        Err(error) => set_error(status, format!("S1F1: {error}")),
    }
}

/// Reply to whatever the equipment reports; never sends a primary.
fn serve_host(
    comm: &Arc<HsmsSsCommunicator>,
    status: &Arc<Mutex<SecsStatus>>,
    stop: &Arc<AtomicBool>,
) {
    while !stop.load(Ordering::SeqCst) {
        match comm.poll_data_message(Duration::from_millis(50)) {
            Ok(Some(message)) => {
                let (stream, function) = (message.get_stream(), message.get_function());
                let result = match (stream, function) {
                    (6, 11) => s6f12(comm, &message, Ackc6::Ok).map(|_| ()),
                    (5, 1) => s5f2(comm, &message, Ackc5::Ok).map(|_| ()),
                    _ => Ok(()),
                };
                match &result {
                    Ok(()) => {
                        let reply = match (stream, function) {
                            (6, 11) => Some("S6F12"),
                            (5, 1) => Some("S5F2"),
                            _ => None,
                        };
                        if let Some(reply) = reply {
                            record(status, "host", reply, "reply sent by the host");
                        }
                    }
                    Err(error) => set_error(status, format!("S{stream}F{function} reply: {error}")),
                }
                record(
                    status,
                    "eq",
                    &format!("S{stream}F{function}"),
                    "primary received by the host",
                );
            }
            Ok(None) => {}
            Err(_) => return,
        }
    }
}

/// S7F3 download, then S7F5/S7F6 read-back with a checksum comparison.
fn upload_recipe(
    comm: &Arc<HsmsSsCommunicator>,
    status: &Arc<Mutex<SecsStatus>>,
    recipe_id: &str,
    checksum: &str,
    body: &str,
) {
    let request = match Secs2::list([
        Secs2::ascii(recipe_id).expect("ppid"),
        Secs2::ascii(body).expect("body"),
    ]) {
        Ok(request) => request,
        Err(error) => {
            set_error(status, format!("S7F3 body: {error}"));
            return;
        }
    };
    match comm.send_data(7, 3, true, request) {
        Ok(Some(reply)) => match Ackc7::from_secs2(reply.secs2()) {
            Ok(Ackc7::Accepted) => {
                {
                    let mut guard = status.lock().expect("status lock");
                    guard.s7f3_uploaded += 1;
                    guard.s7f4_ack += 1;
                }
                record(
                    status,
                    "host",
                    "S7F3",
                    &format!("recipe {recipe_id} accepted"),
                );
            }
            Ok(other) => {
                status.lock().expect("status lock").s7f3_uploaded += 1;
                set_error(status, format!("S7F4 for {recipe_id}: {other:?}"));
                return;
            }
            Err(error) => {
                set_error(status, format!("S7F4 parse: {error}"));
                return;
            }
        },
        Ok(None) => {
            set_error(status, "S7F3 got no reply".to_owned());
            return;
        }
        Err(error) => {
            set_error(status, format!("S7F3: {error}"));
            return;
        }
    }
    // Read it back and compare checksums: the program the tool holds must be
    // the program the host sent.
    let readback = match comm.send_data(7, 5, true, Secs2::ascii(recipe_id).expect("ppid")) {
        Ok(Some(reply)) => reply,
        Ok(None) => {
            set_error(status, "S7F5 got no reply".to_owned());
            return;
        }
        Err(error) => {
            set_error(status, format!("S7F5: {error}"));
            return;
        }
    };
    let returned = readback.secs2().get_ascii_at(&[1]).unwrap_or_default();
    let actual = oxide_core::model::Recipe::checksum_of(&returned);
    if actual == checksum {
        status.lock().expect("status lock").s7f6_verified += 1;
        record(
            status,
            "host",
            "S7F6",
            &format!("recipe {recipe_id} checksum verified: {actual}"),
        );
    } else {
        set_error(
            status,
            format!("S7F6 checksum mismatch for {recipe_id}: {actual} != {checksum}"),
        );
    }
}

// ----------------------------------------------------------------------
// helpers
// ----------------------------------------------------------------------

fn passive_config(config: &SecsConfig) -> HsmsSsCommunicatorConfig {
    let cfg = HsmsSsCommunicatorConfig::new();
    cfg.set_session_id(config.session_id)
        .expect("valid session id");
    cfg.set_connection_mode(secs4rs::hsms::HsmsConnectionMode::Passive);
    cfg.set_socket_address(config.bind);
    cfg.timeout().set_t3(5.0);
    cfg.timeout().set_t6(5.0);
    cfg.timeout().set_t7(10.0);
    cfg.timeout().set_t8(5.0);
    cfg
}

fn active_config(config: &SecsConfig) -> HsmsSsCommunicatorConfig {
    let cfg = HsmsSsCommunicatorConfig::new();
    cfg.set_session_id(config.session_id)
        .expect("valid session id");
    cfg.set_connection_mode(secs4rs::hsms::HsmsConnectionMode::Active);
    cfg.set_socket_address(config.bind);
    cfg.timeout().set_t3(5.0);
    cfg.timeout().set_t5(2.0);
    cfg.timeout().set_t6(5.0);
    cfg.timeout().set_t8(5.0);
    cfg
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn record(status: &Arc<Mutex<SecsStatus>>, from: &str, message: &str, detail: &str) {
    let mut guard = status.lock().expect("status lock");
    guard.traffic.push(TrafficEntry {
        at_ms: now_ms(),
        from: from.to_owned(),
        message: message.to_owned(),
        detail: detail.to_owned(),
    });
    let len = guard.traffic.len();
    if len > 60 {
        guard.traffic.drain(0..len - 60);
    }
}

fn set_error(status: &Arc<Mutex<SecsStatus>>, error: String) {
    let mut guard = status.lock().expect("status lock");
    guard.last_error = Some(error);
}
