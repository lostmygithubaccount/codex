#!/usr/bin/env bash
# demo-channel.sh
#
# End-to-end filesystem-channel demo for the codex external-channel
# feature. Starts codex with a temporary channel directory, sends one
# ping envelope as an external agent, waits for codex to write a
# terminal reply envelope, and prints both sides of the exchange.
#
# Works on macOS (Bash 3.x that ships with the OS) and Linux. Uses
# shell-native tooling only: `mktemp`, `printf`, `date`, `find`,
# `od`, `awk`. No `jq` dependency.
#
# Usage:
#     codex-rs/scripts/demo-channel.sh
#     CODEX_BIN=/path/to/codex codex-rs/scripts/demo-channel.sh
#     TIMEOUT_SECS=60 codex-rs/scripts/demo-channel.sh
#
# Env:
#     CODEX_BIN       codex binary to use (default: `codex` on PATH).
#     TIMEOUT_SECS    how long to wait for the reply (default: 30).
#     KEEP_CHANNEL    if set, do not delete the channel dir on exit.
#
# Exit codes:
#     0  reply observed, envelope parsed
#     1  reply not observed within TIMEOUT_SECS
#     2  codex binary not found
#     3  reply envelope malformed or fields mismatch

set -euo pipefail

CODEX_BIN="${CODEX_BIN:-codex}"
TIMEOUT_SECS="${TIMEOUT_SECS:-30}"

if ! command -v "$CODEX_BIN" >/dev/null 2>&1; then
    printf 'error: codex binary %s not found on PATH\n' "$CODEX_BIN" >&2
    printf 'hint: set CODEX_BIN=/path/to/codex and re-run\n' >&2
    exit 2
fi

CHANNEL_DIR="$(mktemp -d -t codex-channel-demo.XXXXXX)"
INBOX="$CHANNEL_DIR/inbox"
OUTBOX="$CHANNEL_DIR/outbox"
PROCESSED="$CHANNEL_DIR/processed"
mkdir -p "$INBOX" "$OUTBOX" "$PROCESSED"

CODEX_PID=""

cleanup() {
    if [ -n "$CODEX_PID" ] && kill -0 "$CODEX_PID" 2>/dev/null; then
        kill "$CODEX_PID" 2>/dev/null || true
        # Give codex a second to drain before SIGKILL.
        for _ in 1 2 3 4 5; do
            if ! kill -0 "$CODEX_PID" 2>/dev/null; then
                break
            fi
            sleep 0.2
        done
        kill -9 "$CODEX_PID" 2>/dev/null || true
    fi
    if [ -z "${KEEP_CHANNEL:-}" ]; then
        rm -rf "$CHANNEL_DIR"
    else
        printf '\nchannel dir preserved: %s\n' "$CHANNEL_DIR" >&2
    fi
}
trap cleanup EXIT INT TERM

printf 'channel demo\n'
printf '  codex binary : %s\n' "$CODEX_BIN"
printf '  channel dir  : %s\n' "$CHANNEL_DIR"
printf '  timeout      : %ss\n' "$TIMEOUT_SECS"
printf '\n'

# Produce a unique envelope filename following the envelope v1 shape:
# `{nanos:020}-{pid:010}-{rand_hex:08}-{seq:010}-from-{from}.json`.
# `date +%s%N` is GNU-only; macOS `date` does not support nanoseconds.
# We synthesize nanos from seconds * 1_000_000_000 and a random suffix.
envelope_filename() {
    local from="$1"
    local secs
    secs="$(date +%s)"
    local nanos
    nanos="$(awk -v s="$secs" -v r="$RANDOM" 'BEGIN { printf "%020d", s*1000000000 + (r*1000) }')"
    local pid
    pid="$(awk -v p="$$" 'BEGIN { printf "%010d", p }')"
    local rand_hex
    rand_hex="$(awk -v r="$RANDOM" 'BEGIN { printf "%08x", r }')"
    local seq
    seq="$(awk -v r="$RANDOM" 'BEGIN { printf "%010d", r }')"
    printf '%s-%s-%s-%s-from-%s.json' "$nanos" "$pid" "$rand_hex" "$seq" "$from"
}

# Start codex in the background. The exact invocation depends on the
# mode you want to exercise. For the demo we request a non-interactive
# exec-style run; tweak `--` flags to your local codex build.
ENVELOPE_NAME="$(envelope_filename agent0)"
ENVELOPE_PATH="$INBOX/$ENVELOPE_NAME"
TMP_ENVELOPE="$INBOX/.$ENVELOPE_NAME.tmp"

TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
cat >"$TMP_ENVELOPE" <<ENVELOPE
{"id":"demo-1","from":"agent0","to":"codex","kind":"brief","thread":"demo-thread","text":"ping from the channel demo","ts":"$TS"}
ENVELOPE
ln "$TMP_ENVELOPE" "$ENVELOPE_PATH"
rm -f "$TMP_ENVELOPE"
printf '==> wrote inbound envelope\n%s\n\n' "$ENVELOPE_PATH"
cat "$ENVELOPE_PATH"
printf '\n\n'

export CODEX_CHANNEL_DIR="$CHANNEL_DIR"
# Keep codex's stdout+stderr visible; the demo is for humans.
"$CODEX_BIN" &
CODEX_PID=$!

printf '==> codex pid %s, waiting up to %ss for reply...\n' "$CODEX_PID" "$TIMEOUT_SECS"

# Poll outbox for a non-receipt, non-tempfile json that isn't hidden.
DEADLINE=$(( $(date +%s) + TIMEOUT_SECS ))
REPLY_PATH=""
while [ "$(date +%s)" -lt "$DEADLINE" ]; do
    if ! kill -0 "$CODEX_PID" 2>/dev/null; then
        printf 'error: codex exited before writing a reply\n' >&2
        exit 1
    fi
    # Pick the first matching file; sort by name so the nanos-prefixed
    # envelope wins even if a stray file appears.
    REPLY_PATH="$(find "$OUTBOX" -maxdepth 1 -type f -name '*.json' \
        ! -name '.*' ! -name '*.receipt.json' 2>/dev/null | sort | head -n 1)"
    if [ -n "$REPLY_PATH" ] && [ -s "$REPLY_PATH" ]; then
        break
    fi
    sleep 0.5
    REPLY_PATH=""
done

if [ -z "$REPLY_PATH" ]; then
    printf 'error: no reply envelope within %ss\n' "$TIMEOUT_SECS" >&2
    printf 'outbox contents:\n' >&2
    ls -la "$OUTBOX" >&2 || true
    exit 1
fi

printf '\n==> reply envelope\n%s\n\n' "$REPLY_PATH"
cat "$REPLY_PATH"
printf '\n\n'

# Minimal structural check without jq: look for required field markers.
for field in '"from"' '"to"' '"kind"' '"text"' '"ts"'; do
    if ! grep -q "$field" "$REPLY_PATH"; then
        printf 'error: reply envelope missing field %s\n' "$field" >&2
        exit 3
    fi
done

# Assert `from` is `codex` (the replying side).
if ! grep -q '"from":[[:space:]]*"codex"' "$REPLY_PATH"; then
    printf 'error: reply envelope `from` is not "codex"\n' >&2
    exit 3
fi

printf '==> round-trip ok\n'
printf '    inbound : %s\n' "$ENVELOPE_PATH"
printf '    reply   : %s\n' "$REPLY_PATH"
printf '    archive : %s\n' "$PROCESSED"

exit 0
