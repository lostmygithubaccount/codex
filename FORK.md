# Fork notes

This repository is the netsky fork of `openai/codex`.

The fork carries channel support while the work is prepared in an
upstream-friendly form. Core feature names, filenames, and internal
identifiers should describe channels. The installed binary marks itself
as the netsky fork so it is obvious at runtime.

## Fork patches

Patches layered on top of the upstream tag, oldest first:

1. `feat(tui): add filesystem channel support` — add the channel
   subscriber, reply envelopes, docs, demo script, and roundtrip test.
2. `fix(models): use known GPT-5.5 context windows` — prevent stale
   remote model metadata from clamping GPT-5.5 variants below their
   documented context windows.
3. `fix(core): compact before pending input overflows` — account for
   the pending user message before deciding whether to compact.
4. `build(fork): keep netsky install path buildable` — gate WebRTC
   behind a cargo feature, disable it for the fork install path, add
   fork notes, and mark runtime version surfaces as the netsky fork.

## Rebuild ritual

One command from this repository:

```sh
scripts/install-codex-fork.sh
```

That script:

- builds `codex-cli` without default features when WebRTC is not needed
- installs the binary to `${CODEX_INSTALL_DIR:-$HOME/.local/bin}/codex`
- prints the installed binary version so the fork marker is visible

Do not install the fork from a stock upstream checkout. Stock upstream
enables the TUI default features, including `realtime-webrtc`, which
pulls in `webrtc-sys` and can fail on macOS libc++ toolchains. The fork
keeps `codex-cli` and `codex-cloud-tasks` wired to `codex-tui` with
`default-features = false`, so the normal local build path does not
compile WebRTC:

```sh
cd codex-rs
cargo build --release -p codex-cli
install -m 0755 target/release/codex ~/.local/bin/codex
```

If realtime WebRTC is explicitly needed, build with default features and
expect to debug the local C++/SDK toolchain instead of using the normal
fork install path.

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

Current pin: **`rust-v0.128.0`**.

### Upstream tag bump ritual

Run when upstream publishes a new `rust-vX.Y.0`:

```sh
cd /path/to/lostmygithubaccount/codex

# 1. fetch new upstream tags
git fetch upstream --tags

# 2. branch from the new stable tag
NEW=rust-vX.Y.0
git checkout -b rebase/vX.Y.0 "$NEW"

# 3. replay the fork commits in order (list comes from `main` before
# the rebase; adjust if the set changes).
git cherry-pick <commit-1> <commit-2> <commit-3> <commit-4>
# resolve conflicts carefully — every patch has a reason, do not drop
# any of them. Conflicts are usually additive: upstream added a new
# parameter to a function that our patch also modified, and both
# parameters need to coexist.

# 4. build + smoke from this fork checkout
scripts/install-codex-fork.sh
codex --version

# 5. push the rebase branch to fork origin (NOT main; agent0 reviews
# first).
git push origin rebase/vX.Y.0

# 6. after agent0 confirms the build is healthy end-to-end, flip main:
git checkout main
git reset --hard rebase/vX.Y.0
git push --force-with-lease origin main

# 7. update the "Current pin" line above to the new tag.
```

## Upstream posture

Each fork commit should be reviewable as an upstream Codex change:

- no fork-consumer names in Codex code outside the runtime version
  marker, and no docs references except fork identity notes.
- no release metadata changes.
- no fork-only cargo version changes.
- targeted checks on touched Rust packages before pushing.

The channel feature should read as a Codex capability.
