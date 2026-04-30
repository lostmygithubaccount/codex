use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::Utc;
use codex_protocol::config_types::CollaborationModeMask;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use tokio::task::JoinHandle;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::collaboration_modes;
use crate::model_catalog::ModelCatalog;

const CHANNEL_ENV: &str = "CODEX_CHANNEL_DIR";
const MAX_INBOUND_ENVELOPE_BYTES: u64 = 64 * 1024;
const MAX_INBOUND_TEXT_BYTES: usize = 48 * 1024;

// Channel envelope v1. Keep the on-disk shape stable so external
// producers and readers can interoperate without linking to codex-tui.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct Envelope {
    id: Option<String>,
    from: String,
    to: Option<String>,
    kind: Option<String>,
    in_reply_to: Option<String>,
    thread: Option<String>,
    swarm: Option<String>,
    idempotency_key: Option<String>,
    requires_ack: Option<bool>,
    text: String,
    ts: String,
}

#[derive(Debug, Deserialize)]
struct InboundEnvelope {
    id: Option<String>,
    from: Option<String>,
    to: Option<String>,
    kind: Option<String>,
    thread: Option<String>,
    swarm: Option<String>,
    text: Option<String>,
    ts: Option<String>,
}

// Metadata for a submitted inbound envelope that is awaiting a terminal
// reply from codex. The watcher pushes one entry per submit; the TUI event
// hook pops the oldest and writes an outbound envelope.
#[derive(Debug, Clone)]
pub(crate) struct PendingReply {
    in_reply_to: Option<String>,
    to: Option<String>,
    thread: Option<String>,
    swarm: Option<String>,
}

struct ChannelState {
    outbox_dir: PathBuf,
}

static STATE: OnceLock<Mutex<ChannelState>> = OnceLock::new();

fn install_state(outbox_dir: PathBuf) {
    let _ = STATE.set(Mutex::new(ChannelState { outbox_dir }));
}

fn with_state<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut ChannelState) -> R,
{
    let mu = STATE.get()?;
    let mut guard = mu.lock().ok()?;
    Some(f(&mut guard))
}

pub(crate) fn spawn_if_configured(
    app_event_tx: AppEventSender,
    model_catalog: &ModelCatalog,
) -> Option<JoinHandle<()>> {
    let channel_dir = std::env::var_os(CHANNEL_ENV).map(PathBuf::from)?;
    let default_mask = collaboration_modes::default_mode_mask(model_catalog)?;
    install_state(channel_dir.join("outbox"));
    Some(tokio::spawn(async move {
        if let Err(err) = watch_channel_dir(channel_dir, app_event_tx, default_mask).await {
            tracing::warn!("channel watcher stopped: {err}");
        }
    }))
}

async fn watch_channel_dir(
    channel_dir: PathBuf,
    app_event_tx: AppEventSender,
    default_mask: CollaborationModeMask,
) -> io::Result<()> {
    let inbox_dir = channel_dir.join("inbox");
    let outbox_dir = channel_dir.join("outbox");
    let processed_dir = channel_dir.join("processed");
    fs::create_dir_all(&inbox_dir)?;
    fs::create_dir_all(&outbox_dir)?;
    fs::create_dir_all(&processed_dir)?;

    let mut seen = HashSet::new();
    let mut interval = tokio::time::interval(Duration::from_millis(750));
    loop {
        interval.tick().await;
        for path in envelope_paths(&channel_dir, &inbox_dir)? {
            if !seen.insert(path.clone()) {
                continue;
            }
            match consume_envelope(&path, &outbox_dir, &processed_dir) {
                Ok(Some((text, pending))) => {
                    app_event_tx.send(AppEvent::SubmitChannelUserMessage {
                        text,
                        collaboration_mode: default_mask.clone(),
                        pending_reply: pending,
                    });
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!(path = %path.display(), "failed to consume channel envelope: {err}");
                }
            }
        }
    }
}

fn envelope_paths(channel_dir: &Path, inbox_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    collect_json_files(channel_dir, &mut paths)?;
    collect_json_files(inbox_dir, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collect_json_files(dir: &Path, paths: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension() == Some(OsStr::new("json")) {
            paths.push(path);
        }
    }
    Ok(())
}

fn consume_envelope(
    path: &Path,
    outbox_dir: &Path,
    processed_dir: &Path,
) -> io::Result<Option<(String, PendingReply)>> {
    if fs::metadata(path)?.len() > MAX_INBOUND_ENVELOPE_BYTES {
        write_receipt(path, outbox_dir, "rejected_envelope_too_large")?;
        archive(path, processed_dir)?;
        return Ok(None);
    }
    let raw = fs::read_to_string(path)?;
    let envelope: InboundEnvelope = serde_json::from_str(&raw)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let Some(text) = envelope
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
    else {
        write_receipt(path, outbox_dir, "ignored_empty_text")?;
        archive(path, processed_dir)?;
        return Ok(None);
    };
    if text.len() > MAX_INBOUND_TEXT_BYTES {
        write_receipt(path, outbox_dir, "rejected_text_too_large")?;
        archive(path, processed_dir)?;
        return Ok(None);
    }
    let from = envelope.from.as_deref().unwrap_or("external");
    let ts = envelope.ts.as_deref().unwrap_or("unknown");
    // The `to` on an inbound envelope is the recipient (us). When we reply,
    // our `to` is the inbound `from`.
    let pending = PendingReply {
        in_reply_to: envelope.id.clone(),
        to: envelope.from.clone(),
        thread: envelope.thread.clone(),
        swarm: envelope.swarm.clone(),
    };
    write_receipt(path, outbox_dir, "submitted")?;
    archive(path, processed_dir)?;
    let rendered = format!("channel envelope\nfrom: {from}\nts: {ts}\n\n{text}");
    // Silence unused-kind warning until tool_call propagation lands.
    let _ = envelope.kind;
    let _ = envelope.to;
    Ok(Some((rendered, pending)))
}

fn write_receipt(path: &Path, outbox_dir: &Path, status: &str) -> io::Result<()> {
    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("envelope.json");
    let receipt_path = outbox_dir.join(format!("{file_name}.receipt.json"));
    let receipt = json!({
        "from": "codex",
        "status": status,
        "source": file_name,
        "ts": Utc::now().to_rfc3339(),
    });
    let bytes = serde_json::to_vec_pretty(&receipt)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    fs::write(receipt_path, bytes)?;
    Ok(())
}

fn archive(path: &Path, processed_dir: &Path) -> io::Result<()> {
    let Some(file_name) = path.file_name() else {
        return Ok(());
    };
    let archived = processed_dir.join(file_name);
    match fs::rename(path, &archived) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(path, archived)?;
            fs::remove_file(path)
        }
    }
}

// ---------------------------------------------------------------------------
// Outbound emit path
// ---------------------------------------------------------------------------
//
// Policy (matches the brief at briefs/codex-channel-polish-swarm.md):
//   - One terminal envelope per complete assistant turn.
//     kind: "reply", text: last agent message.
//   - Turn with no last_agent_message (tool-call-only turn, no closing prose):
//     write an empty reply so the inbound envelope still receives exactly one
//     terminal outcome.
//   - TurnAborted -> kind: "cancel", text: reason.
//   - Error -> kind: "error", text: error message.
//
// The reply is addressed back to the inbound envelope's `from`, with
// `in_reply_to` set to its id and `thread` / `swarm` propagated verbatim.
// Missing inbound metadata degrades to None fields; the envelope still
// writes so a caller that did not set an id can still observe the reply.

/// Record a completed assistant turn. Called from the TUI event handler on
/// `EventMsg::TurnComplete`. No-op if the channel is not configured.
/// `last_agent_message == None` marks a tool-call-only turn and emits an
/// empty reply.
pub(crate) fn record_turn_complete(pending: PendingReply, last_agent_message: Option<String>) {
    emit_terminal("reply", pending, last_agent_message.unwrap_or_default());
}

/// Record a turn that errored. Called on `EventMsg::Error`.
pub(crate) fn record_turn_error(pending: PendingReply, message: String) {
    emit_terminal("error", pending, message);
}

/// Record a cancelled turn. Called on `EventMsg::TurnAborted`.
pub(crate) fn record_turn_aborted(pending: PendingReply, reason: &str) {
    emit_terminal("cancel", pending, reason.to_owned());
}

pub(crate) fn record_submission_rejected(pending: PendingReply, reason: &str) {
    emit_terminal("error", pending, reason.to_owned());
}

fn emit_terminal(kind: &str, pending: PendingReply, text: String) {
    let Some(outbox_dir) = with_state(|s| s.outbox_dir.clone()) else {
        return;
    };
    if let Err(err) = emit_reply_envelope(&outbox_dir, Some(&pending), kind, &text) {
        tracing::warn!("channel: failed to write {kind} envelope: {err}");
    }
}

fn emit_reply_envelope(
    outbox_dir: &Path,
    pending: Option<&PendingReply>,
    kind: &str,
    text: &str,
) -> io::Result<PathBuf> {
    let env = Envelope {
        id: None,
        from: "codex".to_owned(),
        to: pending.and_then(|p| p.to.clone()),
        kind: Some(kind.to_owned()),
        in_reply_to: pending.and_then(|p| p.in_reply_to.clone()),
        thread: pending.and_then(|p| p.thread.clone()),
        swarm: pending.and_then(|p| p.swarm.clone()),
        idempotency_key: None,
        requires_ack: None,
        text: text.to_owned(),
        ts: Utc::now().to_rfc3339(),
    };
    write_envelope_atomic(outbox_dir, &env)
}

// ---------------------------------------------------------------------------
// Atomic envelope write
// ---------------------------------------------------------------------------
//
// Filename: `{nanos:020}-{pid:010}-{rand_hex:08}-{seq:010}-from-{from}.json`.
// Write flow: serialize -> write `.<name>.<attempt>.tmp` -> hard_link to
// `<name>` (create-new semantics) -> unlink tmp. Retries up to 8 times on
// AlreadyExists with a fresh name. Keep this stable so consumers can
// sort and dedupe across producers.

fn build_filename(from: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let pid = std::process::id();
    let rand_hex: u32 = rand::random();
    format!("{nanos:020}-{pid:010}-{rand_hex:08x}-{seq:010}-from-{from}.json")
}

fn write_envelope_atomic(outbox: &Path, env: &Envelope) -> io::Result<PathBuf> {
    fs::create_dir_all(outbox)?;
    let bytes =
        serde_json::to_vec(env).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut last_err: Option<io::Error> = None;
    for attempt in 0..8 {
        let name = build_filename(&env.from);
        let final_path = outbox.join(&name);
        let tmp_path = outbox.join(format!(".{name}.{attempt}.tmp"));
        fs::write(&tmp_path, &bytes)?;
        match fs::hard_link(&tmp_path, &final_path) {
            Ok(()) => {
                let _ = fs::remove_file(&tmp_path);
                return Ok(final_path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&tmp_path);
                last_err = Some(e);
                continue;
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp_path);
                return Err(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "exhausted 8 attempts to create unique envelope filename",
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_reply(outbox: &Path) -> Envelope {
        let mut hits: Vec<PathBuf> = fs::read_dir(outbox)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                let name = p.file_name().and_then(OsStr::to_str).unwrap_or("");
                name.ends_with(".json")
                    && !name.ends_with(".receipt.json")
                    && !name.starts_with('.')
            })
            .collect();
        hits.sort();
        assert_eq!(
            hits.len(),
            1,
            "expected exactly one reply envelope, got {hits:?}"
        );
        let raw = fs::read_to_string(&hits[0]).unwrap();
        serde_json::from_str(&raw).unwrap()
    }

    #[test]
    fn consume_envelope_extracts_pending_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = tmp.path().join("inbox");
        let outbox = tmp.path().join("outbox");
        let processed = tmp.path().join("processed");
        fs::create_dir_all(&inbox).unwrap();
        fs::create_dir_all(&outbox).unwrap();
        fs::create_dir_all(&processed).unwrap();
        let raw = json!({
            "id": "msg-42",
            "from": "agent0",
            "to": "codex",
            "thread": "thread-1",
            "swarm": "swarm-1",
            "text": "hello codex",
            "ts": "2026-04-17T15:00:00Z",
        });
        let path =
            inbox.join("00000000000000000000-0000000000-00000000-0000000000-from-agent0.json");
        fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();

        let (rendered, pending) = consume_envelope(&path, &outbox, &processed)
            .unwrap()
            .unwrap();
        assert!(rendered.contains("hello codex"));
        assert!(rendered.contains("from: agent0"));
        assert_eq!(pending.in_reply_to.as_deref(), Some("msg-42"));
        assert_eq!(pending.to.as_deref(), Some("agent0"));
        assert_eq!(pending.thread.as_deref(), Some("thread-1"));
        assert_eq!(pending.swarm.as_deref(), Some("swarm-1"));
    }

    #[test]
    fn emit_reply_envelope_writes_canonical_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let outbox = tmp.path().join("outbox");
        let pending = PendingReply {
            in_reply_to: Some("msg-42".into()),
            to: Some("agent0".into()),
            thread: Some("thread-1".into()),
            swarm: Some("swarm-1".into()),
        };
        emit_reply_envelope(&outbox, Some(&pending), "reply", "hi back").unwrap();

        let env = read_reply(&outbox);
        assert_eq!(env.from, "codex");
        assert_eq!(env.to.as_deref(), Some("agent0"));
        assert_eq!(env.kind.as_deref(), Some("reply"));
        assert_eq!(env.in_reply_to.as_deref(), Some("msg-42"));
        assert_eq!(env.thread.as_deref(), Some("thread-1"));
        assert_eq!(env.swarm.as_deref(), Some("swarm-1"));
        assert_eq!(env.text, "hi back");
        // RFC3339 parse sanity
        assert!(chrono::DateTime::parse_from_rfc3339(&env.ts).is_ok());
    }

    #[test]
    fn round_trip_inbox_to_reply() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = tmp.path().join("inbox");
        let outbox = tmp.path().join("outbox");
        let processed = tmp.path().join("processed");
        fs::create_dir_all(&inbox).unwrap();
        fs::create_dir_all(&outbox).unwrap();
        fs::create_dir_all(&processed).unwrap();

        // Drop an inbound envelope.
        let inbound = json!({
            "id": "msg-99",
            "from": "agent0",
            "to": "codex",
            "thread": "t-7",
            "text": "ping",
            "ts": "2026-04-17T15:30:00Z",
        });
        let path =
            inbox.join("00000000000000000001-0000000000-00000001-0000000000-from-agent0.json");
        fs::write(&path, serde_json::to_vec(&inbound).unwrap()).unwrap();

        // Simulate the watcher picking it up and the assistant completing
        // a turn. We do not spin the tokio watcher; we exercise the same
        // seam the watcher uses (consume_envelope -> emit_reply_envelope).
        let (_rendered, pending) = consume_envelope(&path, &outbox, &processed)
            .unwrap()
            .unwrap();
        emit_reply_envelope(&outbox, Some(&pending), "reply", "pong").unwrap();

        let env = read_reply(&outbox);
        assert_eq!(env.from, "codex");
        assert_eq!(env.to.as_deref(), Some("agent0"));
        assert_eq!(env.in_reply_to.as_deref(), Some("msg-99"));
        assert_eq!(env.thread.as_deref(), Some("t-7"));
        assert_eq!(env.kind.as_deref(), Some("reply"));
        assert_eq!(env.text, "pong");
    }

    #[test]
    fn emit_error_and_cancel_kinds() {
        let tmp = tempfile::tempdir().unwrap();
        let outbox = tmp.path().join("outbox");
        let pending = PendingReply {
            in_reply_to: Some("m1".into()),
            to: Some("agent0".into()),
            thread: None,
            swarm: None,
        };
        emit_reply_envelope(&outbox, Some(&pending), "error", "boom").unwrap();
        let paths: Vec<_> = fs::read_dir(&outbox)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        assert_eq!(paths.len(), 1);
        let env: Envelope = serde_json::from_str(&fs::read_to_string(&paths[0]).unwrap()).unwrap();
        assert_eq!(env.kind.as_deref(), Some("error"));
        assert_eq!(env.text, "boom");
    }

    #[test]
    fn emit_reply_without_pending_still_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let outbox = tmp.path().join("outbox");
        emit_reply_envelope(&outbox, None, "reply", "solo").unwrap();
        let env = read_reply(&outbox);
        assert_eq!(env.from, "codex");
        assert_eq!(env.to, None);
        assert_eq!(env.in_reply_to, None);
        assert_eq!(env.thread, None);
        assert_eq!(env.text, "solo");
    }

    #[test]
    fn build_filename_shape_is_stable() {
        let n = build_filename("codex");
        assert!(n.ends_with("-from-codex.json"));
        let parts: Vec<&str> = n.splitn(5, '-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].len(), 20);
        assert_eq!(parts[1].len(), 10);
        assert_eq!(parts[2].len(), 8);
    }
}
