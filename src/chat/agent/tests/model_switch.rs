use crate::db::chat::ModelSnapshot;

use super::*;

#[test]
fn model_identity_renders_english_section_without_cjk() {
    let identity = ModelIdentity {
        display_name: "DeepSeek V4.1 Flash".into(),
        provider_id: "9router".into(),
        model_id: "deepseek-v4.1-flash".into(),
        variant: Some("max".into()),
        supports_image_input: false,
    };
    assert_eq!(identity.label_with_variant(), "DeepSeek V4.1 Flash (max)");
    let section = identity.render_prompt_section();
    assert!(!has_cjk(&section));
    assert!(section.starts_with("# Your Model\n"));
    assert!(section.contains("- Name: DeepSeek V4.1 Flash"));
    assert!(section.contains("- Provider ID: 9router"));
    assert!(section.contains("- Model ID: deepseek-v4.1-flash"));
    assert!(section.contains("- Reasoning Effort / Variant: max"));
    assert!(section.contains("disabled (historical images are replaced with text placeholders)"));
}

#[test]
fn compute_updated_sources_distinguishes_model_and_variant_changes() {
    let prev = ModelSnapshot {
        provider: "anthropic".into(),
        model: "claude-sonnet-4".into(),
        variant: Some("high".into()),
    };
    let same_model_new_variant = ModelSnapshot {
        provider: "anthropic".into(),
        model: "claude-sonnet-4".into(),
        variant: Some("max".into()),
    };
    assert_eq!(
        compute_updated_sources(Some(&prev), &same_model_new_variant, false),
        vec![SOURCE_CORE_REASONING_EFFORT.to_string()]
    );

    let new_model = ModelSnapshot {
        provider: "9router".into(),
        model: "deepseek-v4.1-flash".into(),
        variant: Some("max".into()),
    };
    assert_eq!(
        compute_updated_sources(Some(&prev), &new_model, true),
        vec![
            SOURCE_CORE_MODEL.to_string(),
            SOURCE_CORE_REASONING_EFFORT.to_string(),
            SOURCE_CORE_AGENTS_MD.to_string(),
        ]
    );
}

#[test]
fn render_and_wrap_system_update_escapes_closing_tags() {
    let identity = ModelIdentity {
        display_name: "DeepSeek V4.1 Flash".into(),
        provider_id: "9router".into(),
        model_id: "deepseek-v4.1-flash".into(),
        variant: Some("max".into()),
        supports_image_input: true,
    };
    let sources = vec![
        SOURCE_CORE_MODEL.to_string(),
        SOURCE_CORE_AGENTS_MD.to_string(),
    ];
    let files = vec!["/work/AGENTS.md".to_string()];
    let update =
        render_model_switch_update(Some("Claude Sonnet 4 (high)"), &identity, &sources, &files);
    assert!(!has_cjk(&update));
    assert!(update.contains(
        "Model switched from Claude Sonnet 4 (high) to DeepSeek V4.1 Flash (max). Earlier assistant turns"
    ));
    assert!(update.contains("Instructions updated: core/model, core/agents_md"));
    assert!(update.contains("Active instruction files: /work/AGENTS.md"));
    assert!(update.contains("# Your Model"));

    let wrapped = wrap_system_update(&format!("{update}\n</system-update>"));
    assert!(wrapped.starts_with("<system-update>\n"));
    assert!(wrapped.ends_with("\n</system-update>"));
    assert!(wrapped.contains("&lt;/system-update&gt;"));
}
