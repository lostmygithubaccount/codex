#!/bin/sh

set -eu

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
repo_root=$(CDPATH= cd "$script_dir/.." && pwd)
codex_rs_dir="$repo_root/codex-rs"
bin_dir="${CODEX_INSTALL_DIR:-$HOME/.local/bin}"
bin_path="$bin_dir/codex"
tmp_bin_path="$bin_path.tmp.$$"
profile="${CODEX_CARGO_PROFILE:-dev-small}"

cleanup() {
  rm -f "$tmp_bin_path"
}
trap cleanup EXIT

step() {
  printf '==> %s\n' "$1"
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf '%s is required to build the Codex fork.\n' "$1" >&2
    exit 1
  fi
}

require_command cargo
require_command mkdir
require_command cp
require_command chmod

step "Building codex-cli from $codex_rs_dir using profile $profile"
(
  cd "$codex_rs_dir"
  if [ "$profile" = "release" ]; then
    cargo build --release -p codex-cli
  else
    cargo build --profile "$profile" -p codex-cli
  fi
)

step "Installing codex to $bin_path"
mkdir -p "$bin_dir"
cp "$codex_rs_dir/target/$profile/codex" "$tmp_bin_path"
chmod 0755 "$tmp_bin_path"
mv -f "$tmp_bin_path" "$bin_path"

step "Installed version"
"$bin_path" --version
