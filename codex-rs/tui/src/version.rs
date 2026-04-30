/// The current Codex CLI version as embedded at compile time.
pub const CODEX_CLI_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " [netsky fork https://github.com/lostmygithubaccount/codex]"
);
