use crate::ModelsManagerConfig;
use crate::manager::ModelsManager;
use codex_protocol::openai_models::TruncationPolicyConfig;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::TestModelsEndpoint;
use super::openai_manager_for_tests;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_model_info_without_tool_output_override() {
    let codex_home = TempDir::new().expect("create temp dir");
    let config = ModelsManagerConfig::default();
    let manager = openai_manager_for_tests(
        codex_home.path().to_path_buf(),
        TestModelsEndpoint::new(Vec::new()),
    );

    let model_info = manager.get_model_info("gpt-5.2", &config).await;

    assert_eq!(
        model_info.truncation_policy,
        TruncationPolicyConfig::bytes(/*limit*/ 10_000)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_model_info_with_tool_output_override() {
    let codex_home = TempDir::new().expect("create temp dir");
    let config = ModelsManagerConfig {
        tool_output_token_limit: Some(123),
        ..Default::default()
    };
    let manager = openai_manager_for_tests(
        codex_home.path().to_path_buf(),
        TestModelsEndpoint::new(Vec::new()),
    );

    let model_info = manager.get_model_info("gpt-5.4", &config).await;

    assert_eq!(
        model_info.truncation_policy,
        TruncationPolicyConfig::tokens(/*limit*/ 123)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gpt_5_5_context_window_override_allows_one_million_tokens() {
    let codex_home = TempDir::new().expect("create temp dir");
    let config = ModelsManagerConfig {
        model_context_window: Some(1_000_000),
        ..Default::default()
    };
    let manager = openai_manager_for_tests(
        codex_home.path().to_path_buf(),
        TestModelsEndpoint::new(Vec::new()),
    );

    let model_info = manager.get_model_info("gpt-5.5", &config).await;

    assert_eq!(model_info.context_window, Some(1_000_000));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gpt_5_5_default_context_window_is_one_million_tokens() {
    let codex_home = TempDir::new().expect("create temp dir");
    let manager = openai_manager_for_tests(
        codex_home.path().to_path_buf(),
        TestModelsEndpoint::new(Vec::new()),
    );

    let model_info = manager
        .get_model_info("gpt-5.5", &ModelsManagerConfig::default())
        .await;

    assert_eq!(model_info.context_window, Some(1_000_000));
    assert_eq!(model_info.auto_compact_token_limit(), Some(900_000));
}
