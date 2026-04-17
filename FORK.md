# Fork notes

This repository is a maintained fork of `openai/codex`.

The fork exists so netsky can depend on channel support while the work
is prepared in an upstream-friendly form. The Codex code should stay
generic. Feature names, public docs, commit messages, filenames,
identifiers, and user-facing strings should describe channels, not
netsky.

## Branch policy

- `main`: the only maintained branch on this fork.
- `upstream/main`: fetched from `openai/codex`.
- local topic branches: temporary only. Squash or restack them before
  pushing `main`.

Keep `main` ahead of upstream with a clean linear stack. Rebase onto
`upstream/main` when upstream moves. Push with `--force-with-lease`,
not `--force`.

## Upstream posture

Each fork commit should be reviewable as an upstream Codex change:

- no netsky-specific names in Codex code or docs outside this file.
- no release metadata changes.
- no fork-only cargo version changes.
- targeted checks on touched Rust packages before pushing.

The channel feature should read as a Codex capability. netsky is only a
downstream consumer that needs it early.

## Sync procedure

```sh
git fetch upstream
git fetch origin
git checkout main
git reset --hard upstream/main
git cherry-pick <fork commits>
cargo check -p codex -p codex-core -p codex-tui -p codex-mcp -p codex-rmcp-client
cargo test --no-run -p codex-core -p codex-tui
git push --force-with-lease origin main
```

The workspace-level build may fail on local `webrtc-sys` toolchain or
SDK issues. Treat targeted package gates as the fork gate unless the
channel stack touches the failing dependency path.
