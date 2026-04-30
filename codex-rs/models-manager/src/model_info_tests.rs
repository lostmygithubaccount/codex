use super::*;
use crate::ModelsManagerConfig;
use pretty_assertions::assert_eq;

#[test]
fn reasoning_summaries_override_true_enables_support() {
    let model = model_info_from_slug("unknown-model");
    let config = ModelsManagerConfig {
        model_supports_reasoning_summaries: Some(true),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);
    let mut expected = model;
    expected.supports_reasoning_summaries = true;

    assert_eq!(updated, expected);
}

#[test]
fn reasoning_summaries_override_false_does_not_disable_support() {
    let mut model = model_info_from_slug("unknown-model");
    model.supports_reasoning_summaries = true;
    let config = ModelsManagerConfig {
        model_supports_reasoning_summaries: Some(false),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);

    assert_eq!(updated, model);
}

#[test]
fn reasoning_summaries_override_false_is_noop_when_model_is_false() {
    let model = model_info_from_slug("unknown-model");
    let config = ModelsManagerConfig {
        model_supports_reasoning_summaries: Some(false),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);

    assert_eq!(updated, model);
}

#[test]
fn model_context_window_override_clamps_to_max_context_window() {
    let mut model = model_info_from_slug("unknown-model");
    model.context_window = Some(273_000);
    model.max_context_window = Some(400_000);
    let config = ModelsManagerConfig {
        model_context_window: Some(500_000),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);
    let mut expected = model;
    expected.context_window = Some(400_000);

    assert_eq!(updated, expected);
}

#[test]
fn model_context_window_uses_model_value_without_override() {
    let mut model = model_info_from_slug("unknown-model");
    model.context_window = Some(273_000);
    model.max_context_window = Some(400_000);
    let config = ModelsManagerConfig::default();

    let updated = with_config_overrides(model.clone(), &config);

    assert_eq!(updated, model);
}

#[test]
fn gpt_5_5_context_window_override_ignores_stale_remote_maximum() {
    let mut model = model_info_from_slug("gpt-5.5");
    model.context_window = Some(272_000);
    model.max_context_window = Some(272_000);
    let config = ModelsManagerConfig {
        model_context_window: Some(1_000_000),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);
    let mut expected = model;
    expected.context_window = Some(1_000_000);
    expected.max_context_window = Some(1_000_000);

    assert_eq!(updated, expected);
}

#[test]
fn gpt_5_5_default_context_window_ignores_stale_remote_metadata() {
    let mut model = model_info_from_slug("gpt-5.5");
    model.context_window = Some(272_000);
    model.max_context_window = Some(272_000);

    let updated = with_config_overrides(model.clone(), &ModelsManagerConfig::default());
    let mut expected = model;
    expected.context_window = Some(1_000_000);
    expected.max_context_window = Some(1_000_000);

    assert_eq!(updated, expected);
}

#[test]
fn gpt_5_5_pro_default_context_window_uses_pro_limit() {
    let mut model = model_info_from_slug("gpt-5.5-pro");
    model.context_window = Some(272_000);
    model.max_context_window = Some(272_000);

    let updated = with_config_overrides(model.clone(), &ModelsManagerConfig::default());
    let mut expected = model;
    expected.context_window = Some(1_050_000);
    expected.max_context_window = Some(1_050_000);

    assert_eq!(updated, expected);
}

#[test]
fn gpt_5_5_context_window_override_clamps_to_known_model_limit() {
    let mut model = model_info_from_slug("gpt-5.5");
    model.context_window = Some(272_000);
    model.max_context_window = Some(272_000);
    let config = ModelsManagerConfig {
        model_context_window: Some(1_050_000),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);
    let mut expected = model;
    expected.context_window = Some(1_000_000);
    expected.max_context_window = Some(1_000_000);

    assert_eq!(updated, expected);
}

#[test]
fn gpt_5_5_pro_context_window_override_allows_pro_limit() {
    let mut model = model_info_from_slug("gpt-5.5-pro");
    model.context_window = Some(272_000);
    model.max_context_window = Some(272_000);
    let config = ModelsManagerConfig {
        model_context_window: Some(1_050_000),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);
    let mut expected = model;
    expected.context_window = Some(1_050_000);
    expected.max_context_window = Some(1_050_000);

    assert_eq!(updated, expected);
}
