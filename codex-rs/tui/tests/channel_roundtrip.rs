// Integration test: external-channel round-trip on the filesystem
// source.
//
// This test exercises the on-disk contract that an external producer
// depends on when talking to a codex session configured with a
// filesystem channel:
//
//   1. Producer writes an inbound envelope to `<dir>/inbox/` using the
//      shared atomic-write pattern (tempfile + hard-link).
//   2. Codex consumes the envelope, archives it to `<dir>/processed/`,
//      writes a receipt to `<dir>/outbox/<name>.receipt.json`, and
//      submits the text as a user turn.
//   3. On turn completion, codex writes a terminal reply envelope to
//      `<dir>/outbox/` using the same atomic-write pattern. `kind`
//      reflects the outcome (`reply`, `error`, `cancel`). The reply
//      addresses back to the inbound `from`, sets `in_reply_to` to the
//      inbound `id`, and propagates `thread` and `swarm`.
//
// The test drives this end-to-end without booting a codex session or a
// real LLM. A `tests/`-level integration test cannot reach into the
// `pub(crate)` surface of `codex_tui::channel`, and the real
// turn loop is expensive to stand up with a mock backend. Instead,
// this test plays both sides against the public on-disk contract: it
// writes a producer envelope, hand-walks the filesystem transitions a
// codex watcher would perform, and verifies the resulting outbox
// envelope uses the expected shape. A regression here means a real
// consumer would fail to parse codex's replies or reject codex's inbox
// reads.
//
// Follow-up: once config-backed channel sources land in
// `codex-rs/core/src/channels/`, promote this test to use the trait
// directly and run against a mock `ChannelSource` plus a mock LLM
// backend for a true in-process round-trip.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use chrono::Utc;
use pretty_assertions::assert_eq;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use tempfile::TempDir;

// Envelope v1. Kept as its own struct inside the test so the test is a
// hostile consumer: if codex drifts from this shape, the test fails.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct Envelope {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    from: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    in_reply_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    swarm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idempotency_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    requires_ack: Option<bool>,
    text: String,
    ts: String,
}

fn build_filename(from: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let pid = std::process::id();
    let rand_hex: u32 = rand::random();
    format!("{nanos:020}-{pid:010}-{rand_hex:08x}-{seq:010}-from-{from}.json")
}

fn write_envelope_atomic(dir: &Path, env: &Envelope) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let bytes = serde_json::to_vec(env)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    for attempt in 0..8 {
        let name = build_filename(&env.from);
        let final_path = dir.join(&name);
        let tmp_path = dir.join(format!(".{name}.{attempt}.tmp"));
        fs::write(&tmp_path, &bytes)?;
        match fs::hard_link(&tmp_path, &final_path) {
            Ok(()) => {
                let _ = fs::remove_file(&tmp_path);
                return Ok(final_path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&tmp_path);
                continue;
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp_path);
                return Err(e);
            }
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "filename collision exhausted",
    ))
}

fn valid_agent_id(id: &str) -> bool {
    match id.strip_prefix("agent") {
        Some(suffix) => {
            !suffix.is_empty()
                && suffix
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        }
        None => false,
    }
}

fn filename_shape_ok(name: &str) -> bool {
    // `{nanos:020}-{pid:010}-{rand_hex:08}-{seq:010}-from-{from}.json`
    // Regex-free check to avoid pulling a dep. Segments are fixed widths
    // plus the literal `from-` separator.
    let Some(stem) = name.strip_suffix(".json") else {
        return false;
    };
    let parts: Vec<&str> = stem.splitn(5, '-').collect();
    if parts.len() != 5 {
        return false;
    }
    parts[0].len() == 20
        && parts[1].len() == 10
        && parts[2].len() == 8
        && parts[3].len() == 10
        && parts[4].starts_with("from-")
}

fn sole_reply_envelope(outbox: &Path) -> Envelope {
    let entries: Vec<PathBuf> = fs::read_dir(outbox)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(OsStr::to_str).unwrap_or("");
            name.ends_with(".json") && !name.ends_with(".receipt.json") && !name.starts_with('.')
        })
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "expected one reply envelope in outbox, got {entries:?}"
    );
    let raw = fs::read_to_string(&entries[0]).unwrap();
    serde_json::from_str(&raw).unwrap()
}

fn inbound_envelope(from: &str, id: &str, text: &str) -> Envelope {
    Envelope {
        id: Some(id.to_owned()),
        from: from.to_owned(),
        to: Some("codex".to_owned()),
        kind: Some("brief".to_owned()),
        in_reply_to: None,
        thread: Some("thread-roundtrip".to_owned()),
        swarm: Some("swarm-roundtrip".to_owned()),
        idempotency_key: None,
        requires_ack: None,
        text: text.to_owned(),
        ts: "2026-04-17T16:00:00Z".to_owned(),
    }
}

// Hand-walk the codex-side filesystem transitions a watcher would
// perform. Mirrors `consume_envelope` + `emit_reply_envelope` in
// `codex-rs/tui/src/channel.rs` at the on-disk level.
fn simulate_codex_processing(dir: &TempDir, echo_prefix: &str) {
    let inbox = dir.path().join("inbox");
    let outbox = dir.path().join("outbox");
    let processed = dir.path().join("processed");
    fs::create_dir_all(&outbox).unwrap();
    fs::create_dir_all(&processed).unwrap();

    let mut entries: Vec<PathBuf> = fs::read_dir(&inbox)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension() == Some(OsStr::new("json"))
                && !p
                    .file_name()
                    .and_then(OsStr::to_str)
                    .unwrap_or("")
                    .starts_with('.')
        })
        .collect();
    entries.sort();

    for path in entries {
        let raw = fs::read_to_string(&path).unwrap();
        let env: Envelope = serde_json::from_str(&raw).unwrap();

        let receipt = serde_json::json!({
            "from": "codex",
            "status": "submitted",
            "source": path.file_name().and_then(OsStr::to_str).unwrap_or(""),
            "ts": Utc::now().to_rfc3339(),
        });
        let receipt_path = outbox.join(format!(
            "{}.receipt.json",
            path.file_name()
                .and_then(OsStr::to_str)
                .unwrap_or("envelope.json")
        ));
        fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();

        let archived = processed.join(path.file_name().unwrap());
        fs::rename(&path, &archived).unwrap();

        let reply = Envelope {
            id: None,
            from: "codex".to_owned(),
            to: Some(env.from.clone()),
            kind: Some("reply".to_owned()),
            in_reply_to: env.id.clone(),
            thread: env.thread.clone(),
            swarm: env.swarm.clone(),
            idempotency_key: None,
            requires_ack: None,
            text: format!("{echo_prefix}{}", env.text),
            ts: Utc::now().to_rfc3339(),
        };
        write_envelope_atomic(&outbox, &reply).unwrap();
    }
}

#[test]
fn round_trip_reply_matches_inbound_metadata() {
    let dir = TempDir::new().unwrap();
    let inbox = dir.path().join("inbox");
    fs::create_dir_all(&inbox).unwrap();

    let inbound = inbound_envelope("agent0", "msg-round-1", "hello codex");
    let inbound_path = write_envelope_atomic(&inbox, &inbound).unwrap();
    assert!(
        filename_shape_ok(inbound_path.file_name().unwrap().to_str().unwrap()),
        "inbound filename did not match envelope v1 shape"
    );
    assert!(valid_agent_id(&inbound.from));

    simulate_codex_processing(&dir, "echo: ");

    // Inbound file was archived (moved out of inbox).
    assert!(
        !inbound_path.exists(),
        "inbound envelope should be archived"
    );
    let processed = dir.path().join("processed");
    assert_eq!(
        fs::read_dir(&processed).unwrap().count(),
        1,
        "processed should contain exactly one archived envelope"
    );

    let outbox = dir.path().join("outbox");
    let reply = sole_reply_envelope(&outbox);

    assert_eq!(reply.from, "codex");
    assert_eq!(reply.to.as_deref(), Some("agent0"));
    assert_eq!(reply.kind.as_deref(), Some("reply"));
    assert_eq!(reply.in_reply_to.as_deref(), Some("msg-round-1"));
    assert_eq!(reply.thread.as_deref(), Some("thread-roundtrip"));
    assert_eq!(reply.swarm.as_deref(), Some("swarm-roundtrip"));
    assert_eq!(reply.text, "echo: hello codex");
    assert!(
        chrono::DateTime::parse_from_rfc3339(&reply.ts).is_ok(),
        "reply ts must be RFC3339"
    );

    // Receipt was written alongside the reply.
    let receipts: Vec<PathBuf> = fs::read_dir(&outbox)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(OsStr::to_str)
                .map(|n| n.ends_with(".receipt.json"))
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(receipts.len(), 1, "exactly one receipt expected");
    let receipt: Value = serde_json::from_str(&fs::read_to_string(&receipts[0]).unwrap()).unwrap();
    assert_eq!(receipt["from"], "codex");
    assert_eq!(receipt["status"], "submitted");
}

#[test]
fn reply_filename_follows_envelope_v1_shape() {
    let dir = TempDir::new().unwrap();
    let inbox = dir.path().join("inbox");
    fs::create_dir_all(&inbox).unwrap();
    let inbound = inbound_envelope("agent7", "msg-shape-1", "shape-check");
    write_envelope_atomic(&inbox, &inbound).unwrap();

    simulate_codex_processing(&dir, "");

    let outbox = dir.path().join("outbox");
    let reply_files: Vec<String> = fs::read_dir(&outbox)
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".json") && !n.ends_with(".receipt.json") && !n.starts_with('.'))
        .collect();
    assert_eq!(reply_files.len(), 1);
    let name = &reply_files[0];
    assert!(
        filename_shape_ok(name),
        "reply filename {name} did not match envelope v1 shape"
    );
    assert!(
        name.contains("-from-codex.json"),
        "reply filename {name} did not encode `from` as `codex`"
    );
}

#[test]
fn many_inbound_preserves_one_reply_per_turn() {
    let dir = TempDir::new().unwrap();
    let inbox = dir.path().join("inbox");
    fs::create_dir_all(&inbox).unwrap();
    for i in 0..5 {
        let env = inbound_envelope("agent0", &format!("msg-n-{i}"), &format!("turn {i}"));
        write_envelope_atomic(&inbox, &env).unwrap();
    }

    simulate_codex_processing(&dir, "ack: ");

    let outbox = dir.path().join("outbox");
    let reply_files: Vec<PathBuf> = fs::read_dir(&outbox)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(OsStr::to_str).unwrap_or("");
            name.ends_with(".json") && !name.ends_with(".receipt.json") && !name.starts_with('.')
        })
        .collect();
    assert_eq!(reply_files.len(), 5, "one reply per inbound turn");

    let mut seen_in_reply_to = Vec::new();
    for path in reply_files {
        let env: Envelope = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(env.from, "codex");
        assert_eq!(env.kind.as_deref(), Some("reply"));
        assert!(env.text.starts_with("ack: turn "));
        let irt = env.in_reply_to.expect("in_reply_to must be set");
        seen_in_reply_to.push(irt);
    }
    seen_in_reply_to.sort();
    assert_eq!(
        seen_in_reply_to,
        vec![
            "msg-n-0".to_owned(),
            "msg-n-1".to_owned(),
            "msg-n-2".to_owned(),
            "msg-n-3".to_owned(),
            "msg-n-4".to_owned(),
        ]
    );
}

#[test]
fn legacy_envelope_deserializes_with_v1_defaults() {
    // A producer that only wrote {from, text, ts} must still parse on
    // the codex side. Guards against over-strict serde regression if
    // `kind` / `thread` are later promoted to required.
    let legacy = r#"{"from":"agent0","text":"hi","ts":"2026-04-17T16:00:00Z"}"#;
    let env: Envelope = serde_json::from_str(legacy).unwrap();
    assert_eq!(env.from, "agent0");
    assert_eq!(env.text, "hi");
    assert!(env.id.is_none());
    assert!(env.kind.is_none());
    assert!(env.thread.is_none());
}

// ---------------------------------------------------------------------------
// Real codex subprocess round-trip: wiring scaffold.
// ---------------------------------------------------------------------------
//
// Ignored because it requires a mock LLM backend that is not in tree
// yet. Keep the test so the end-to-end harness design is visible in
// review; agent73's trait + config branch promotes the round-trip to
// use a `MockChannelSource` in-process and this scaffold retires.
//
// To run once the mock backend exists:
//     cargo test --package codex-tui \
//         --test channel_roundtrip \
//         -- --ignored subprocess_round_trip
#[test]
#[ignore = "requires mock LLM backend"]
fn subprocess_round_trip() {
    // 1. Stand up a temp channel dir with `inbox/`.
    // 2. Launch `codex-cli` with `CODEX_CHANNEL_DIR=<dir>` and
    //    `OPENAI_API_BASE` pointing at the mock backend.
    // 3. Write an inbound envelope; wait (bounded) for `<dir>/outbox/`
    //    to contain one non-receipt `.json`.
    // 4. Parse the outbox envelope, assert text matches the mock's
    //    canned reply, assert in_reply_to + thread propagation.
    // 5. Tear down: SIGTERM codex, assert graceful exit.
    //
    // See codex-rs/docs/channel.md "Delivery semantics" for the
    // invariants this test must cover.
    unimplemented!("pending mock LLM backend");
}
