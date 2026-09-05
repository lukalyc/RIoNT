//! Async NT4 client task.
//!
//! NT4 over WebSocket (port 5810, subprotocol "networktables.first.wpi.edu"):
//! - Text frames: JSON control messages `{"method":..,"params":..}`
//! - Binary frames: MessagePack 4-tuples `[topicId|pubuid, timestamp_us, type, value]`
//!
//! The UI talks to this task exclusively through the `NtUpdate` /
//! `ClientCommand` channels, so the render loop never blocks on the network.

use crate::nt::store::{NtType, NtValue};
use futures_util::{SinkExt, StreamExt};
use rmpv::encode::write_value;
use rmpv::Value as Mv;
use serde_json::json;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message as WsMessage;

pub const NT_PORT: u16 = 5810;
const NT_SUBPROTOCOL: &str = "networktables.first.wpi.edu";

/// NT4 data type codes (see networktables4.adoc "Supported Data Types").
const DT_BOOLEAN: u64 = 0;
const DT_DOUBLE: u64 = 1;
const DT_INT: u64 = 2;
const DT_FLOAT: u64 = 3;
const DT_STRING: u64 = 4; // also json
const DT_BINARY: u64 = 5; // raw / msgpack / rpc / protobuf
const DT_BOOLEAN_ARRAY: u64 = 16;
const DT_DOUBLE_ARRAY: u64 = 17;
const DT_INT_ARRAY: u64 = 18;
const DT_FLOAT_ARRAY: u64 = 19;
const DT_STRING_ARRAY: u64 = 20;

/// Things the client tells the UI.
#[derive(Debug, Clone)]
pub enum NtUpdate {
    /// Trying to reach `target`; `attempt` counts retries since the last
    /// successful connection (0 = first try).
    Connecting { target: String, attempt: u32 },
    Connected { server_info: String },
    Disconnected(String),
    /// (topic, value, server_timestamp_us) batch.
    Values(Vec<(String, NtValue, u64)>),
    /// Topic metadata arrived/changed. `None` fields mean "unchanged".
    TopicMeta {
        name: String,
        id: u64,
        data_type: Option<NtType>,
        persistent: Option<bool>,
        retained: Option<bool>,
        /// Wire type string from announce (e.g. "struct:Pose2d").
        type_str: Option<String>,
        /// Advertised structSchema property (e.g. "Pose2d{...}").
        struct_schema: Option<String>,
    },
    /// Topic deleted on the server.
    TopicRemoved(String),
    /// Background job result from the client task (e.g. SSH restart).
    Toast {
        kind: crate::app::ToastKind,
        msg: String,
    },
    // The client measures these for clock-synced publishes; the current
    // driver-station HUD intentionally does not surface raw rtt/offset.
    #[allow(dead_code)]
    /// Round trip time in ms.
    Rtt(f64),
    #[allow(dead_code)]
    /// Best estimate of (server_time - local_epoch_time) in us.
    ClockOffset(f64),
}

/// Things the UI asks the client to do.
#[derive(Debug, Clone)]
pub enum ClientCommand {
    /// Publish a value to a topic (declares the topic if needed).
    Publish { topic: String, value: NtValue },
    /// Drop the socket and reconnect (pure client action: no NT queries).
    Reconnect,
    /// Drop the socket and connect to a different target instead.
    Retarget(String),
    /// Dispatch a background SSH command that restarts the robot code.
    RestartRobotCode { host: String, user: String, cmd: String },
}

pub fn channel() -> (UnboundedSender<NtUpdate>, UnboundedReceiver<NtUpdate>) {
    unbounded_channel()
}

pub fn command_channel() -> (UnboundedSender<ClientCommand>, UnboundedReceiver<ClientCommand>) {
    unbounded_channel()
}

/// Local epoch time in us (server uses epoch us off-robot, FPGA us on robot).
fn local_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

/// Fire-and-forget SSH restart of the robot user code. Runs entirely in a
/// background task; the outcome lands back in the UI as a toast. Never
/// touches NetworkTables — the command comes straight from config.json
/// (`system.ssh_user` + `system.restart_cmd`).
fn spawn_restart(
    updates: &UnboundedSender<NtUpdate>,
    host: String,
    user: String,
    cmd: String,
) {
    let updates = updates.clone();
    tokio::spawn(async move {
        let started = Instant::now();
        let output = tokio::process::Command::new("ssh")
            // tokio's output() leaves stdin INHERITED (unlike std), which
            // would hand the TUI's keystroke pipe to ssh and hang it.
            .stdin(std::process::Stdio::null())
            .arg("-o")
            .arg("ConnectTimeout=5")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("StrictHostKeyChecking=no")
            .arg(format!("{}@{}", user, host))
            .arg(&cmd)
            .output()
            .await;
        let elapsed = started.elapsed().as_secs_f32();
        match output {
            Ok(out) if out.status.success() => {
                updates
                    .send(NtUpdate::Toast {
                        kind: crate::app::ToastKind::Success,
                        msg: format!("robot code restarted in {:.2}s", elapsed),
                    })
                    .ok();
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let detail = stderr.lines().last().unwrap_or("ssh failed").trim().to_string();
                updates
                    .send(NtUpdate::Toast {
                        kind: crate::app::ToastKind::Error,
                        msg: format!("restart failed: {}", detail),
                    })
                    .ok();
            }
            Err(e) => {
                updates
                    .send(NtUpdate::Toast {
                        kind: crate::app::ToastKind::Error,
                        msg: format!("restart: ssh unavailable ({})", e),
                    })
                    .ok();
            }
        }
    });
}

// ---------------------------------------------------------------------------
// debug logging (RIONT_DEBUG=1)
// ---------------------------------------------------------------------------

type DebugLog = Option<std::io::BufWriter<std::fs::File>>;

fn open_debug_log() -> DebugLog {
    if std::env::var("RIONT_DEBUG").map(|v| v == "1").unwrap_or(false) {
        std::fs::File::create("riont-debug.log")
            .ok()
            .map(std::io::BufWriter::new)
    } else {
        None
    }
}

fn debug_msg(log: &mut DebugLog, line: &str) {
    if let Some(f) = log {
        use std::io::Write;
        let _ = writeln!(f, "{}", line);
        let _ = f.flush();
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

// ---------------------------------------------------------------------------
// client loop
// ---------------------------------------------------------------------------

/// A publish whose value frame may need retransmission (the server drops
/// the first value frame for a freshly-published topic).
struct Pending {
    frame: Vec<u8>,
    last_sent: Instant,
    tries: u32,
}

/// Reconnects forever until the process exits.
pub async fn run_client(
    mut target: String,
    updates: UnboundedSender<NtUpdate>,
    commands_tx: UnboundedSender<ClientCommand>,
    mut commands: UnboundedReceiver<ClientCommand>,
) {
    if !target.contains(':') {
        target = format!("{}:{}", target, NT_PORT);
    }
    // Set by Retarget commands (from the in-app connect overlay); applied at
    // the top of the next loop iteration.
    let retarget_to: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let mut attempt: u32 = 0;
    // Suppress duplicate-failure toasts: an unreachable target retries with
    // the SAME reason every ~6.5s forever, which would churn the UI's
    // 4-slot stack and evict everything else. A Disconnected update fires
    // only when the reason changes (or after a session that had actually
    // connected — a fresh drop after a live link is new information). The
    // HUD keeps the persistent surface (reason + attempt count).
    let mut last_reason: Option<String> = None;
    loop {
        updates
            .send(NtUpdate::Connecting { target: target.clone(), attempt })
            .ok();
        let outcome = session(&target, &updates, &mut commands, &commands_tx, &retarget_to).await;
        // Ok = the session got connected before dying; Err = it never did
        // (connect-phase failure). The reason string is the same either way.
        let (reason, had_connected) = match outcome {
            Ok(reason) => (reason, true),
            Err(e) => (e.to_string(), false),
        };
        // A deliberate retarget, manual reconnect, or restart-during-connect
        // is not a disconnect; don't report them as one so the UI's HUD
        // state stays up.
        if reason != "retarget" && reason != "reconnect requested" && reason != "restart during connect"
            && last_reason.as_deref() != Some(reason.as_str())
        {
            updates.send(NtUpdate::Disconnected(reason.clone())).ok();
        }
        last_reason = Some(reason);
        if had_connected {
            last_reason = None;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(1500)) => {}
            Some(cmd) = commands.recv() => match cmd {
                ClientCommand::Reconnect => {}
                ClientCommand::Retarget(t) => *retarget_to.lock().unwrap() = Some(t),
                // Restart works even while disconnected (SSH does not need
                // the NT link).
                ClientCommand::RestartRobotCode { host, user, cmd } => {
                    spawn_restart(&updates, host, user, cmd);
                }
                // A publish that lands in the backoff window must not be
                // lost: re-queue it for the next session.
                pub_cmd @ ClientCommand::Publish { .. } => {
                    commands_tx.send(pub_cmd).ok();
                }
            },
        }
        if let Some(t) = retarget_to.lock().unwrap().take() {
            target = if t.contains(':') { t } else { format!("{}:{}", t, NT_PORT) };
            attempt = 0; // new target, fresh retry count
        } else {
            attempt = attempt.saturating_add(1);
        }
    }
}

/// One full session over WebSocket. Returns Ok(reason) if the session had
/// been connected when it died, Err(reason) if it never got past the
/// connect phase — the reason string is the death cause either way.
async fn session(
    target: &str,
    updates: &UnboundedSender<NtUpdate>,
    commands: &mut UnboundedReceiver<ClientCommand>,
    commands_tx: &UnboundedSender<ClientCommand>,
    retarget_to: &Arc<Mutex<Option<String>>>,
) -> Result<String, String> {
    let mut debug_log = open_debug_log();
    debug_msg(&mut debug_log, &format!("session start: {}", target));

    // NOTE: the NT4 WebSocket is served at /nt/<client-name>.
    let url = format!("ws://{}/nt/riont", target);
    let mut request = url
        .into_client_request()
        .map_err(|e| format!("bad request: {}", e))?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_static(NT_SUBPROTOCOL),
    );

    // Connect with a hard timeout, and keep servicing commands while
    // connecting: an unreachable target otherwise hangs for the full TCP
    // timeout (~20s on Windows) and a Retarget typed meanwhile sits queued,
    // making the UI look permanently stuck.
    let connect_fut = tokio_tungstenite::connect_async(request);
    let (ws, _resp) = tokio::select! {
        res = connect_fut => res.map_err(|e| format!("connect: {}", e))?,
        _ = tokio::time::sleep(Duration::from_secs(5)) => {
            return Err("connect timeout".into());
        }
        Some(cmd) = commands.recv() => match cmd {
            ClientCommand::Reconnect => return Err("reconnect requested".into()),
            ClientCommand::Retarget(t) => {
                *retarget_to.lock().unwrap() = Some(t);
                return Err("retarget".into());
            }
            // SSH restart does not need the NT link: fire it. Aborting the
            // in-flight handshake is fine — the outer loop reconnects.
            ClientCommand::RestartRobotCode { host, user, cmd } => {
                spawn_restart(updates, host, user, cmd);
                return Err("restart during connect".into());
            }
            pub_cmd @ ClientCommand::Publish { .. } => {
                commands_tx.send(pub_cmd).ok();
                return Err("connect interrupted".into());
            }
        },
    };
    let (mut sink, mut stream) = ws.split();
    debug_msg(&mut debug_log, "ws connected");
    // Ok/Err contract with run_client: Ok means "this session had been
    // connected when it died", so the caller can reset its dedupe state.
    // Deferred init: every path that reads it passes the assignment below.
    let was_connected;

    // ---- subscribe to everything, all values, fast periodic.
    let subscribe = json!([{
        "method": "subscribe",
        "params": {
            "topics": [""],
            "subuid": 1,
            "options": {
                "prefix": true,
                "all": true,
                "periodic": 0.02
            }
        }
    }]);
    sink.send(WsMessage::Text(subscribe.to_string().into()))
        .await
        .map_err(|e| format!("send subscribe: {}", e))?;
    debug_msg(&mut debug_log, "subscribe sent");
    // Every path from here on had a live link; early returns above never
    // reach the Ok/Err tail that reads this.
    was_connected = true;
    updates
        .send(NtUpdate::Connected {
            server_info: "NT4 server".into(),
        })
        .ok();

    // topic id -> name (assigned by announce messages)
    let mut topic_by_id: HashMap<u64, String> = HashMap::new();
    let mut buf_values: Vec<(String, NtValue, u64)> = Vec::new();
    let mut next_pubuid: u64 = 1;
    // Timers MUST be created once and ticked inside the select: recreating a
    // sleep future per loop iteration means a busy socket (one frame per
    // value, several hundred/s on a real robot) never lets the 50 ms sleep
    // finish and the flush/RTT arms starve forever.
    let mut flush = tokio::time::interval(Duration::from_millis(50));
    let mut rtt = tokio::time::interval(Duration::from_millis(1000));
    // server_time ≈ local_time + offset (from RTT echo).
    let mut clock_offset_us: f64 = 0.0;
    let mut clock_synced = false;
    // Pending publishes: the server drops the first value frame for a
    // freshly-published topic, so retransmit the value frame a few times.
    // Confirmed when the server's `announce` for our publish arrives (it
    // echoes our pubuid, proving the publisher binding is live).
    let mut pending: Vec<Pending> = Vec::new();

    let reason = loop {
        tokio::select! {
            msg = stream.next() => {
                let wsmsg = match msg {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => break format!("ws: {}", e),
                    None => break "ws closed".into(),
                };
                match wsmsg {
                    WsMessage::Text(t) => {
                        if let Err(e) = handle_text(&t, &mut topic_by_id, &mut buf_values, updates, &mut debug_log, &mut pending) {
                            debug_msg(&mut debug_log, &format!("text handle error: {}", e));
                        }
                    }
                    WsMessage::Binary(b) => {
                        // One frame may contain several MessagePack messages.
                        let mut data: &[u8] = &b;
                        while !data.is_empty() {
                            let val = match rmpv::decode::read_value(&mut data) {
                                Ok(v) => v,
                                Err(e) => {
                                    debug_msg(&mut debug_log, &format!("msgpack decode error: {}", e));
                                    break;
                                }
                            };
                            handle_mp(&val, &topic_by_id, &mut buf_values, updates, &mut debug_log, &mut clock_offset_us, &mut clock_synced);
                        }
                    }
                    WsMessage::Ping(p) => {
                        sink.send(WsMessage::Pong(p)).await.ok();
                    }
                    WsMessage::Close(_) => break "server closed".into(),
                    _ => {}
                }
            }
            _ = rtt.tick() => {
                // RTT / clock sync: echo request with topic id -1. The server
                // returns the message with its timestamp filled in.
                let now = local_us();
                let rtt_msg = vec![
                    Mv::Integer((-1i64).into()),
                    Mv::Integer(0.into()),
                    Mv::Integer(2.into()),
                    Mv::Integer((now as i64).into()),
                ];
                let mut buf = Vec::new();
                if write_value(&mut buf, &Mv::Array(rtt_msg)).is_ok() {
                    if sink.send(WsMessage::Binary(buf.into())).await.is_err() {
                        break "write rtt".into();
                    }
                }
            }
            _ = flush.tick() => {
                // Retransmit value frames for unconfirmed publishes: the
                // server drops the first one for a fresh pubuid.
                if !pending.is_empty() {
                    let mut resend: Vec<(usize, Vec<u8>)> = Vec::new();
                    for (i, p) in pending.iter().enumerate() {
                        if p.tries < 3 && p.last_sent.elapsed() >= Duration::from_millis(300) {
                            resend.push((i, p.frame.clone()));
                        }
                    }
                    for (i, frame) in resend {
                        if sink.send(WsMessage::Binary(frame.into())).await.is_ok() {
                            pending[i].tries += 1;
                            pending[i].last_sent = Instant::now();
                        }
                    }
                }
                if !buf_values.is_empty() {
                    let batch = std::mem::take(&mut buf_values);
                    updates.send(NtUpdate::Values(batch)).ok();
                }
            }
            Some(cmd) = commands.recv() => {
                match cmd {
                    ClientCommand::Reconnect => break "reconnect requested".into(),
                    ClientCommand::Retarget(t) => {
                        *retarget_to.lock().unwrap() = Some(t);
                        break "retarget".into();
                    }
                    ClientCommand::RestartRobotCode { host, user, cmd } => {
                        spawn_restart(updates, host, user, cmd);
                    }
                    ClientCommand::Publish { topic, value } => {
                        // Wire-format topic names are absolute.
                        let wire_topic = if topic.starts_with('/') {
                            topic.clone()
                        } else {
                            format!("/{}", topic)
                        };
                        let pubuid = next_pubuid;
                        next_pubuid += 1;
                        let (type_str, dt, mv) = to_msgpack(&value);
                        // 1) declare the publisher (also creates the topic)
                        let publish = json!([{
                            "method": "publish",
                            "params": {
                                "name": wire_topic,
                                "pubuid": pubuid,
                                "type": type_str,
                                "properties": {}
                            }
                        }]);
                        // 2) send the value itself, timestamped in SERVER time
                        // (per spec, values before clock sync use ts=0).
                        let ts = if clock_synced {
                            (local_us() as f64 + clock_offset_us) as i64
                        } else {
                            0
                        };
                        let val_msg = vec![
                            Mv::Integer(pubuid.into()),
                            Mv::Integer(ts.into()),
                            Mv::Integer(dt.into()),
                            mv,
                        ];
                        let mut bin = Vec::new();
                        let ok = write_value(&mut bin, &Mv::Array(val_msg)).is_ok();
                        debug_msg(&mut debug_log, &format!(
                            "publish {} = {:?} json={} bin_hex={}",
                            topic,
                            value,
                            publish.to_string(),
                            hex(&bin)
                        ));
                        if sink.send(WsMessage::Text(publish.to_string().into())).await.is_ok()
                            && ok
                            && sink.send(WsMessage::Binary(bin.clone().into())).await.is_ok()
                        {
                            pending.push(Pending {
                                frame: bin,
                                last_sent: Instant::now(),
                                tries: 1,
                            });
                        } else {
                            break "write publish".into();
                        }
                    }
                }
            }
        }
    };

    if !buf_values.is_empty() {
        updates.send(NtUpdate::Values(buf_values)).ok();
    }
    if was_connected {
        Ok(reason)
    } else {
        Err(reason)
    }
}

/// ---------------------------------------------------------------------------
// JSON control messages
// ---------------------------------------------------------------------------

fn handle_text(
    t: &str,
    topic_by_id: &mut HashMap<u64, String>,
    _buf_values: &mut Vec<(String, NtValue, u64)>,
    updates: &UnboundedSender<NtUpdate>,
    log: &mut DebugLog,
    _pending: &mut Vec<Pending>,
) -> Result<(), String> {
    let msgs: serde_json::Value = serde_json::from_str(t).map_err(|e| e.to_string())?;
    let items: Vec<serde_json::Value> = if msgs.is_array() {
        msgs.as_array().unwrap().clone()
    } else {
        vec![msgs]
    };
    for item in items {
        let method = item.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = item.get("params").cloned().unwrap_or(json!({}));
        match method {
            "announce" => {
                // Store keys use display paths without the leading slash.
                let raw = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let name = raw.strip_prefix('/').unwrap_or(raw).to_string();
                let id = params.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                let type_str = params.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let props = params.get("properties").cloned().unwrap_or(json!({}));
                debug_msg(log, &format!("announce id={} name={} type={}", id, name, type_str));
                // Announce carrying our pubuid = server confirmed our publish.
                // NOTE: do NOT cancel retransmissions here — the server still
                // drops the first VALUE frame for a fresh pubuid; the pending
                // entry ages out after its retries.
                topic_by_id.insert(id, name.clone());
                updates.send(NtUpdate::TopicMeta {
                    name,
                    id,
                    data_type: Some(NtType::from_str(type_str)),
                    persistent: props.get("persistent").and_then(|v| v.as_bool()),
                    retained: props.get("retained").and_then(|v| v.as_bool()),
                    type_str: if type_str.is_empty() {
                        None
                    } else {
                        Some(type_str.to_string())
                    },
                    struct_schema: props
                        .get("structSchema")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                })
                .ok();
            }
            "unannounce" => {
                let raw = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let name = raw.strip_prefix('/').unwrap_or(raw).to_string();
                let id = params.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                debug_msg(log, &format!("unannounce id={} name={}", id, name));
                topic_by_id.remove(&id);
                updates.send(NtUpdate::TopicRemoved(name)).ok();
            }
            "properties" => {
                let raw = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let name = raw.strip_prefix('/').unwrap_or(raw).to_string();
                let update = params.get("update").cloned().unwrap_or(json!({}));
                debug_msg(log, &format!("properties name={} update={}", name, update));
                updates.send(NtUpdate::TopicMeta {
                    name,
                    id: u64::MAX, // unknown; id unchanged
                    data_type: None,
                    persistent: update.get("persistent").and_then(|v| v.as_bool()),
                    retained: update.get("retained").and_then(|v| v.as_bool()),
                    type_str: None,
                    struct_schema: update
                        .get("structSchema")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                })
                .ok();
            }
            other => {
                debug_msg(log, &format!("ignore text method={}", other));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// MessagePack value frames: [id, timestamp_us, type, value]
// ---------------------------------------------------------------------------

fn handle_mp(
    val: &Mv,
    topic_by_id: &HashMap<u64, String>,
    buf_values: &mut Vec<(String, NtValue, u64)>,
    updates: &UnboundedSender<NtUpdate>,
    log: &mut DebugLog,
    clock_offset_us: &mut f64,
    clock_synced: &mut bool,
) {
    let arr = match val.as_array() {
        Some(a) if a.len() == 4 => a,
        _ => return,
    };
    let id = int_of(&arr[0]);
    let ts = int_of(&arr[1]).max(0) as u64;
    let dt = int_of(&arr[2]);

    // RTT echo: server returns our message with its timestamp in element 1.
    if id == -1 {
        let client_sent = match &arr[3] {
            Mv::F64(f) => *f,
            Mv::Integer(i) => i.as_i64().unwrap_or(0) as f64,
            _ => 0.0,
        };
        if client_sent > 0.0 {
            let now = local_us() as f64;
            let rtt_us = now - client_sent;
            if rtt_us >= 0.0 {
                updates.send(NtUpdate::Rtt(rtt_us / 1000.0)).ok();
                let offset = ts as f64 + rtt_us / 2.0 - now;
                updates.send(NtUpdate::ClockOffset(offset)).ok();
                *clock_offset_us = offset;
                *clock_synced = true;
                debug_msg(log, &format!("rtt echo: {:.1}ms", rtt_us / 1000.0));
            }
        }
        return;
    }

    let Some(name) = topic_by_id.get(&(id as u64)) else {
        debug_msg(log, &format!("value for unknown topic id {}", id));
        return;
    };
    debug_msg(log, &format!("value name={} dt={} val={:?}", name, dt, arr[3]));
    let Some(value) = nt_value(dt as u64, &arr[3]) else {
        debug_msg(log, &format!("undecodable value type {} for {}", dt, name));
        return;
    };
    buf_values.push((name.clone(), value, ts));
}

fn int_of(v: &Mv) -> i64 {
    match v {
        Mv::Integer(i) => i.as_i64().unwrap_or(0),
        _ => 0,
    }
}

fn nt_value(dt: u64, v: &Mv) -> Option<NtValue> {
    match dt {
        DT_BOOLEAN => v.as_bool().map(NtValue::Boolean),
        DT_DOUBLE | DT_FLOAT => match v {
            Mv::F64(f) => Some(NtValue::Double(*f)),
            Mv::F32(f) => Some(NtValue::Double(*f as f64)),
            Mv::Integer(i) => Some(NtValue::Double(i.as_i64()? as f64)),
            _ => None,
        },
        DT_INT => v.as_i64().map(NtValue::Int),
        DT_STRING => match v {
            Mv::String(s) => s.as_str().map(|s| NtValue::Str(s.to_string())),
            Mv::Binary(b) => Some(NtValue::Json(String::from_utf8_lossy(b).into_owned())),
            _ => None,
        },
        DT_BINARY => match v {
            Mv::Binary(b) => Some(NtValue::Raw(b.clone())),
            Mv::String(s) => s.as_str().map(|s| NtValue::Raw(s.as_bytes().to_vec())),
            _ => None,
        },
        DT_BOOLEAN_ARRAY => v.as_array().map(|a| {
            NtValue::BooleanArray(a.iter().filter_map(|x| x.as_bool()).collect())
        }),
        DT_DOUBLE_ARRAY | DT_FLOAT_ARRAY => v.as_array().map(|a| {
            NtValue::DoubleArray(
                a.iter()
                    .filter_map(|x| match x {
                        Mv::F64(f) => Some(*f),
                        Mv::F32(f) => Some(*f as f64),
                        Mv::Integer(i) => i.as_i64().map(|i| i as f64),
                        _ => None,
                    })
                    .collect(),
            )
        }),
        DT_INT_ARRAY => v.as_array().map(|a| {
            NtValue::IntArray(a.iter().filter_map(|x| x.as_i64()).collect())
        }),
        DT_STRING_ARRAY => v.as_array().map(|a| {
            NtValue::StringArray(
                a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect(),
            )
        }),
        _ => match v {
            Mv::Binary(b) => Some(NtValue::Raw(b.clone())),
            _ => None,
        },
    }
}

/// Map an NtValue to (type string, msgpack type code, msgpack value).
fn to_msgpack(v: &NtValue) -> (&'static str, u64, Mv) {
    match v {
        NtValue::Boolean(b) => ("boolean", DT_BOOLEAN, Mv::Boolean(*b)),
        NtValue::Double(f) => ("double", DT_DOUBLE, Mv::F64(*f)),
        NtValue::Int(i) => ("int", DT_INT, Mv::Integer((*i).into())),
        NtValue::Str(s) => ("string", DT_STRING, Mv::String(s.clone().into())),
        NtValue::BooleanArray(a) => (
            "boolean[]",
            DT_BOOLEAN_ARRAY,
            Mv::Array(a.iter().map(|b| Mv::Boolean(*b)).collect()),
        ),
        NtValue::DoubleArray(a) => (
            "double[]",
            DT_DOUBLE_ARRAY,
            Mv::Array(a.iter().map(|f| Mv::F64(*f)).collect()),
        ),
        NtValue::IntArray(a) => (
            "int[]",
            DT_INT_ARRAY,
            Mv::Array(a.iter().map(|i| Mv::Integer((*i).into())).collect()),
        ),
        NtValue::StringArray(a) => (
            "string[]",
            DT_STRING_ARRAY,
            Mv::Array(a.iter().map(|s| Mv::String(s.clone().into())).collect()),
        ),
        NtValue::Json(s) => ("json", DT_STRING, Mv::String(s.clone().into())),
        NtValue::Raw(b) => ("msgpack", DT_BINARY, Mv::Binary(b.clone())),
        // Pose2d values are never user-published (not writable), but a
        // programmatic round-trip encodes the canonical 24-byte LE struct
        // payload so the wire format stays valid.
        NtValue::Pose2d { x, y, radians } => {
            let mut b = Vec::with_capacity(24);
            b.extend_from_slice(&x.to_le_bytes());
            b.extend_from_slice(&y.to_le_bytes());
            b.extend_from_slice(&radians.to_le_bytes());
            ("struct:Pose2d", DT_BINARY, Mv::Binary(b))
        }
    }
}

// silence unused import warning for Cursor if codecs change
#[allow(unused)]
fn _cursor_unused(c: Cursor<Vec<u8>>) -> Cursor<Vec<u8>> {
    c
}
