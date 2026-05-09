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
2. `fix(core): compact before pending input overflows` — account for
   the pending user message before deciding whether to compact.
3. `build(fork): keep netsky install path buildable` — gate WebRTC
   behind a cargo feature, disable it for the fork install path, add
   fork notes, and mark runtime version surfaces as the netsky fork.
4. `fix(core): compact before steered input overflows` — compact before
   mid-turn user input is recorded when that pending input would cross
   the auto-compact limit.
5. `build(fork): refresh lockfile for v0.130.0` — refresh package
   versions after rebasing onto the upstream stable tag.
6. `build(fork): default install to dev-small profile` — avoid the
   expensive release fat-LTO path for normal local fork installs.

## GPT-5.5 context window

Do not hardcode GPT-5.5 to a 1M context window for Codex ChatGPT-auth
sessions.

OpenAI advertises `gpt-5.5` in the API with a 1,050,000-token context
window:

- https://developers.openai.com/api/docs/models/gpt-5.5
- https://openai.com/index/introducing-gpt-5-5/

That is not the same surface as Codex running through ChatGPT auth. The
GPT-5.5 launch post says GPT-5.5 in Codex has a 400K context window, and
the ChatGPT help article describes GPT-5.5 Thinking on Pro as 272K input
+ 128K max output:

- https://help.openai.com/en/articles/11909943-gpt-53-and-52-in-chatgpt

Local probes through this fork's installed `codex` binary confirmed the
Codex/ChatGPT-auth behavior:

- a single prompt over 1,048,576 characters is rejected by Codex turn
  start before model inference.
- with a temporary model catalog forcing `gpt-5.5` to 1,050,000 tokens,
  cumulative prompts around 259K tokens succeeded.
- adding roughly another 30K tokens failed with
  `context_length_exceeded`.

The fork therefore keeps bundled `gpt-5.5` metadata at `272_000` input
tokens. Raising this should be gated behind an explicit API-key profile
or a runtime probe that verifies the active provider/auth path actually
accepts the larger window.

## Rebuild ritual

One command from this repository:

```sh
scripts/install-codex-fork.sh
```

That script:

- builds `codex-cli` with the `dev-small` profile by default
- installs the binary to `${CODEX_INSTALL_DIR:-$HOME/.local/bin}/codex`
- prints the installed binary version so the fork marker is visible

Set `CODEX_CARGO_PROFILE=release` when an optimized release binary is
needed. The upstream release profile uses fat LTO and can take a long
time to link locally.

Do not install the fork from a stock upstream checkout. Stock upstream
enables the TUI default features, including `realtime-webrtc`, which
pulls in `webrtc-sys` and can fail on macOS libc++ toolchains. The fork
keeps `codex-cli` and `codex-cloud-tasks` wired to `codex-tui` with
`default-features = false`, so the normal local build path does not
compile WebRTC:

```sh
cd codex-rs
cargo build --profile dev-small -p codex-cli
install -m 0755 target/dev-small/codex ~/.local/bin/codex
```

If realtime WebRTC is explicitly needed, build with default features and
expect to debug the local C++/SDK toolchain instead of using the normal
fork install path.

## Branch policy

- `main`: the current source of truth on this fork. Rebased onto the
  newest stable upstream release tag after build + smoke clear.
- `release/vX.Y.Z`: historical fork tips for upstream stable releases.
  Keep one release branch per published fork update so older fork states
  remain easy to inspect.
- `upstream/main`: fetched from `openai/codex`. Reference only, not the
  rebase target.
- local topic branches: temporary only. Squash or restack them before
  pushing `main`.

### Tracking target

The fork tracks upstream stable release tags (`rust-vX.Y.Z`, no
`-alpha.N`). A tagged release is a stable surface; `upstream/main` can
carry mid-refactor state that breaks our rebase unpredictably.

Current pin: **`rust-v0.130.0`**.

### Upstream tag bump ritual

Run when upstream publishes a new `rust-vX.Y.0`:

```sh
cd /path/to/lostmygithubaccount/codex

# 1. fetch new upstream tags
git fetch upstream --tags

# 2. preserve the current fork tip if its historical release branch does
# not already exist
OLD=release/vX.Y.Z
git branch "$OLD" main
git push origin "$OLD"

# 3. branch from the new stable tag
NEW=rust-vX.Y.0
REL=release/vX.Y.0
git checkout -b "$REL" "$NEW"

# 4. replay the fork commits in order (list comes from `main` before
# the rebase; adjust if the set changes).
git cherry-pick <commit-1> <commit-2> <commit-3> <commit-4>
# resolve conflicts carefully — every patch has a reason, do not drop
# any of them. Conflicts are usually additive: upstream added a new
# parameter to a function that our patch also modified, and both
# parameters need to coexist.

# 5. update the "Current pin" line above and this ritual if branch
# policy changes.

# 6. build + smoke from this fork checkout
scripts/install-codex-fork.sh
codex --version

# 7. publish the historical release branch and make main the latest
# fork source of truth.
git push origin "$REL"
git checkout main
git reset --hard "$REL"
git push --force-with-lease origin main
```

## Upstream posture

Each fork commit should be reviewable as an upstream Codex change:

- no fork-consumer names in Codex code outside the runtime version
  marker, and no docs references except fork identity notes.
- no release metadata changes.
- no fork-only cargo version changes.
- targeted checks on touched Rust packages before pushing.

The channel feature should read as a Codex capability.
