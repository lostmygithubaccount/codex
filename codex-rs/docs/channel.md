# Codex channels

Codex channels let the TUI accept turns from an external process and
emit a terminal outcome for each turn. A channel is a bidirectional
attachment point for tools that need to drive a Codex session without
typing into the terminal.

The current implementation provides a filesystem channel behind an
environment variable. MCP channel notifications can be forwarded into
the session event stream. Config-backed channel sources can build on
the same envelope and delivery rules.

## Enable a filesystem channel

Set `CODEX_CHANNEL_DIR` before starting the TUI:

```sh
export CODEX_CHANNEL_DIR=/tmp/codex-channel
codex
```

Codex creates three subdirectories under the channel directory:

- `inbox/`: producers write inbound envelope files here.
- `outbox/`: codex writes receipts and terminal outcome envelopes here.
- `processed/`: codex moves consumed inbound files here.

For local development, codex also scans json files placed directly in
the channel directory. New producers should write to `inbox/`.

## Envelope shape

One inbound or outbound turn is a json file:

| field | type | required | meaning |
| --- | --- | --- | --- |
| `id` | string | no | stable id assigned by the producer |
| `from` | string | yes | producer id |
| `to` | string | no | recipient id |
| `kind` | string | no | `brief`, `reply`, `error`, or `cancel` |
| `in_reply_to` | string | no | id of the turn this envelope answers |
| `thread` | string | no | conversation id, propagated inbound to outbound |
| `swarm` | string | no | group id, propagated inbound to outbound |
| `idempotency_key` | string | no | retry-safe dedupe key |
| `requires_ack` | bool | no | producer wants an ack, not just a reply |
| `text` | string | yes | model-visible payload |
| `ts` | string | yes | RFC3339 timestamp |

Filenames follow this shape:

```text
{nanos:020}-{pid:010}-{rand_hex:08}-{seq:010}-from-{from}.json
```

Readers sort filenames lexicographically to get best-effort timestamp
order. Writers should create files atomically by writing a temporary
file, hard-linking it to the final filename, then deleting the
temporary file.

## Delivery semantics

- Inbound is at least once. A producer may redeliver after a crash.
- Codex keeps a per-process seen set for filesystem paths.
- Codex writes a receipt for every consumed envelope.
- Codex writes one terminal envelope for a completed turn.
- Tool-call-only turns do not emit a reply envelope.
- Interrupted turns emit `kind: "cancel"`.
- Failed turns emit `kind: "error"`.
- Successful turns emit `kind: "reply"`.

Receipts confirm submission into the TUI event path. They do not
represent the model's final answer. Terminal envelopes carry the final
turn outcome.

## Example

Start codex:

```sh
export CODEX_CHANNEL_DIR=/tmp/codex-channel
codex
```

Write an inbound envelope:

```sh
mkdir -p /tmp/codex-channel/inbox
cat >/tmp/codex-channel/inbox/example.json <<'JSON'
{
  "id": "msg-1",
  "from": "driver",
  "to": "codex",
  "kind": "brief",
  "thread": "demo",
  "text": "Say hello in one sentence.",
  "ts": "2026-04-17T16:00:00Z"
}
JSON
```

Codex renders the turn as a channel envelope, archives the inbound
file under `processed/`, writes `example.json.receipt.json`, then
writes one terminal envelope under `outbox/` when the turn ends.

## MCP notifications

MCP channel notifications use the same session event path. Codex
receives channel notifications from MCP clients, forwards them through
the session, and submits the content as a user turn.

Channel notification support is intentionally transport-neutral. MCP
servers own their authentication and authorization. Codex treats an
accepted channel event as equivalent to stdin.

## Demo

Run:

```sh
codex-rs/scripts/demo-channel.sh
```

The demo starts codex with a temporary channel directory, writes one
inbound envelope, waits for a terminal reply envelope, and prints both
sides. It uses shell-native tools only.

Useful environment variables:

- `CODEX_BIN`: codex binary to run. Defaults to `codex`.
- `TIMEOUT_SECS`: seconds to wait for a reply. Defaults to `30`.
- `KEEP_CHANNEL`: keep the temporary channel directory after exit.

## Tests

The filesystem contract test lives at
`codex-rs/tui/tests/channel_roundtrip.rs`. It writes producer
envelopes, simulates codex processing, and verifies receipts, terminal
envelopes, metadata propagation, and filename shape.

Run:

```sh
cd codex-rs
cargo test --no-run -p codex-tui --test channel_roundtrip
```

The unit tests in `codex-rs/tui/src/channel.rs` cover envelope
consumption, receipt writes, terminal outcome writes, and filename
generation.
