//! 模型切换 `<system-update>` 注入与系统提示词 `# Your Model` / `AGENTS.md` 装配测试。

use crate::db::chat::ModelSwitchRecord;

use super::*;

fn switch_record(sequence: i64, to_label: &str, prompt: &str) -> ModelSwitchRecord {
    ModelSwitchRecord {
        id: format!("msw-{sequence}"),
        sequence,
        from_provider: Some("anthropic".into()),
        from_model: Some("claude-sonnet-4".into()),
        from_variant: Some("high".into()),
        to_provider: "9router".into(),
        to_model: "deepseek-v4.1-flash".into(),
        to_variant: Some("max".into()),
        from_label: Some("Claude Sonnet 4 (high)".into()),
        to_label: to_label.into(),
        updated_sources: vec!["core/model".into()],
        injected_prompt: prompt.into(),
        created_at_ms: 1_700_000_000_000,
    }
}

#[test]
fn model_switch_injects_system_update_in_sequence_order_before_draft() {
    let messages = vec![
        message("u0", 0, 0, "user", "complete", "turn 1 question"),
        message("a0", 1, 0, "assistant", "complete", "turn 1 answer"),
        message("u1", 3, 1, "user", "complete", "turn 2 question"),
        message("a1", 4, 1, "assistant", "complete", "turn 2 answer"),
    ];
    let switches = vec![
        switch_record(2, "DeepSeek V4.1 Flash", "Switched before turn 2"),
        switch_record(5, "GPT-5", "Switched before draft"),
    ];
    let assembly = assemble(&AssemblyInput {
        messages: &messages,
        draft: "current draft",
        draft_attachments: &[],
        model: &model(),
        context_limit: None,
        output_limit: None,
        variant_params: None,
        compact: None,
        model_identity: Some("# Your Model\n- Name: GPT-5"),
        environment: Some("<environment_context/>"),
        instructions: Some(
            "# AGENTS.md instructions for /work\n\n<INSTRUCTIONS>\nUse rtk\n</INSTRUCTIONS>",
        ),
        skills: None,
        mcp: None,
        model_switches: &switches,
        skill_blocks: &[],
        supports_image_input: true,
    });

    assert!(
        assembly
            .system_prompt
            .contains("# Your Model\n- Name: GPT-5")
    );
    assert!(
        assembly
            .system_prompt
            .contains("# AGENTS.md instructions for /work")
    );
    assert_eq!(
        assembly.request.items,
        vec![
            ModelItem::UserText("turn 1 question".into()),
            ModelItem::AssistantText("turn 1 answer".into()),
            ModelItem::UserText("<system-update>\nSwitched before turn 2\n</system-update>".into()),
            ModelItem::UserText("turn 2 question".into()),
            ModelItem::AssistantText("turn 2 answer".into()),
            ModelItem::UserText("<system-update>\nSwitched before draft\n</system-update>".into()),
            ModelItem::UserText("current draft".into()),
        ]
    );
}

#[test]
fn compact_checkpoint_filters_earlier_model_switches() {
    let messages = vec![
        message("u0", 0, 0, "user", "complete", "turn 1"),
        message("a0", 1, 0, "assistant", "complete", "ans 1"),
        message("u1", 4, 1, "user", "complete", "turn 2"),
        message("a1", 5, 1, "assistant", "complete", "ans 2"),
    ];
    let switches = vec![
        switch_record(2, "Old Switch", "Before compact"),
        switch_record(6, "New Switch", "After compact"),
    ];
    let compact = CompactContext {
        end_sequence: 3,
        summary: "compacted summary".into(),
        model_label: "DeepSeek".into(),
    };
    let assembly = assemble(&AssemblyInput {
        messages: &messages,
        draft: "next",
        draft_attachments: &[],
        model: &model(),
        context_limit: None,
        output_limit: None,
        variant_params: None,
        compact: Some(&compact),
        model_identity: None,
        environment: None,
        instructions: None,
        skills: None,
        mcp: None,
        model_switches: &switches,
        skill_blocks: &[],
        supports_image_input: true,
    });

    let serialized = format!("{:?}", assembly.request.items);
    assert!(!serialized.contains("Before compact"));
    assert!(serialized.contains("After compact"));
}
