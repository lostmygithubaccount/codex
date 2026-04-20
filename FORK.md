# Fork notes

This repository is a maintained fork of `openai/codex`.

The fork exists so netsky can depend on channel support while the work
is prepared in an upstream-friendly form. The Codex code should stay
generic. Feature names, public docs, commit messages, filenames,
identifiers, and user-facing strings should describe channels, not
netsky.

## Netsky patches

Eight commits layered on top of the upstream tag, oldest first:

1. `feat(tui): make realtime WebRTC optional` — gate the WebRTC stack
   behind a cargo feature so downstream builds can skip `webrtc-sys` on
   toolchains where it breaks (macOS libc++).
2. `feat(mcp): add channel notification support` — MCP + rmcp-client
   plumbing for an out-of-band notification stream.
3. `feat(tui): add external channel subscriber` — TUI-side subscriber
   wired to the MCP channel stream.
4. `feat(tui): emit reply envelopes on turn completion` — emit
   structured reply envelopes when a TUI turn completes.
5. `test(tui): add channel roundtrip integration test` — end-to-end
   test for the channel pipe.
6. `docs: add channel guide` — user-facing channel docs + demo script.
7. `docs: add fork notes` — this file.
8. `docs: note intent to track stable release tags once feasible` —
   the policy that led to the current `rust-v0.122.0` pin.

## Rebuild ritual

One command from inside the netsky repo:

```sh
scripts/install-codex-fork.sh
```

That script:

- clones or reuses `gh-org/lostmygithubaccount/codex` on whatever branch
  is checked out (expected: `main`)
- patches `codex-cli` and `codex-cloud-tasks` to depend on `codex-tui`
  with `default-features = false` (local-only; never committed) to skip
  the `webrtc-sys` path on macOS
- runs `CARGO_PROFILE_RELEASE_LTO=false
  CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 cargo build --release -p
  codex-cli`
- installs the binary to `/opt/homebrew/bin/codex` and writes sidecar
  metadata at `/opt/homebrew/bin/codex.fork`

## Branch policy

- `main`: the only maintained branch on this fork. Rebased onto stable
  upstream release tags.
- `upstream/main`: fetched from `openai/codex`. Reference only, not the
  rebase target.
- `rebase/vX.Y.Z`: the branch carrying a candidate rebase onto a new
  upstream stable tag. Fast-forwarded into `main` after build + smoke
  clear.
- local topic branches: temporary only. Squash or restack them before
  pushing `main`.

### Tracking target

The fork tracks upstream stable release tags (`rust-vX.Y.Z`, no
`-alpha.N`). A tagged release is a stable surface; `upstream/main` can
carry mid-refactor state that breaks our rebase unpredictably.

Current pin: **`rust-v0.122.0`**.

### Upstream tag bump ritual

Run when upstream publishes a new `rust-vX.Y.0`:

```sh
cd gh-org/lostmygithubaccount/codex

# 1. fetch new upstream tags
git fetch upstream --tags

# 2. branch from the new stable tag
NEW=rust-vX.Y.0
git checkout -b rebase/vX.Y.0 "$NEW"

# 3. replay the netsky commits in order (list comes from `main` before
# the rebase; adjust if the set changes).
git cherry-pick \
  <commit-1> <commit-2> <commit-3> <commit-4> \
  <commit-5> <commit-6> <commit-7> <commit-8>
# resolve conflicts carefully — every patch has a reason, do not drop
# any of them. Conflicts are usually additive: upstream added a new
# parameter to a function that our patch also modified, and both
# parameters need to coexist.

# 4. build + smoke from inside netsky
cd /path/to/netsky
scripts/install-codex-fork.sh
codex --version

# 5. push the rebase branch to fork origin (NOT main; agent0 reviews
# first).
cd gh-org/lostmygithubaccount/codex
git push origin rebase/vX.Y.0

# 6. after agent0 confirms the build is healthy end-to-end, flip main:
git checkout main
git reset --hard rebase/vX.Y.0
git push --force-with-lease origin main

# 7. update the "Current pin" line above to the new tag.
```

## Upstream posture

Each fork commit should be reviewable as an upstream Codex change:

- no netsky-specific names in Codex code or docs outside this file.
- no release metadata changes.
- no fork-only cargo version changes.
- targeted checks on touched Rust packages before pushing.

The channel feature should read as a Codex capability. netsky is only a
downstream consumer that needs it early.
