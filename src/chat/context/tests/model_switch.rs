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
fn trimmed_history_retains_latest_prior_model_switch_and_accounts_for_system_prompt_budget() {
    // 每轮正文 ≈ 1000 token；历史预算只够一轮，三轮合计远超。
    let long_text = |label: &str| format!("{label}{}", "x".repeat(4_000));
    let first = long_text("turn 1 ");
    let second = long_text("turn 2 ");
    let third = long_text("turn 3 ");
    let messages = vec![
        message("u0", 0, 0, "user", "complete", &first),
        message("a0", 1, 0, "assistant", "complete", "ok"),
        message("u1", 3, 1, "user", "complete", &second),
        message("a1", 4, 1, "assistant", "complete", "ok"),
        message("u2", 6, 2, "user", "complete", &third),
        message("a2", 7, 2, "assistant", "complete", "ok"),
    ];
    let switches = vec![
        switch_record(2, "Earlier Model", "Earlier switch"),
        switch_record(5, "Latest Prior Model", "Latest prior switch"),
    ];
    let environment = "<environment_context/>";
    let instructions = format!(
        "# AGENTS.md instructions\n\n<INSTRUCTIONS>\n{}\n</INSTRUCTIONS>",
        "r".repeat(40_000)
    );
    // 与 `assemble` 同一口径算出动态系统提示词（AGENTS.md）占用：本测试的预算只够一轮，正是因为它被扣除。
    let system_tokens = super::super::estimate_tokens(&instructions) as u64;
    let history_budget = 1_256;
    let context_limit = system_tokens
        + super::super::ENVELOPE_RESERVE_TOKENS
        + super::super::USER_SHELL_INJECTION_RESERVE_TOKENS
        + history_budget;

    let assembly = assemble(&AssemblyInput {
        messages: &messages,
        draft: "current draft",
        draft_attachments: &[],
        model: &model(),
        context_limit: Some(context_limit),
        output_limit: Some(0),
        variant_params: None,
        compact: None,
        model_identity: None,
        environment: Some(environment),
        instructions: Some(&instructions),
        skills: None,
        mcp: None,
        model_switches: &switches,
        skill_blocks: &[],
        supports_image_input: true,
    });

    // 系统提示词（≈13k token）计入预算后只剩一轮；不计入时三轮都会保留。
    assert_eq!(
        assembly.omitted_turns, 2,
        "系统提示词开销必须从历史预算中扣除"
    );
    assert_eq!(
        assembly.request.items,
        vec![
            ModelItem::UserText("<system-update>\nLatest prior switch\n</system-update>".into()),
            ModelItem::UserText(third),
            ModelItem::AssistantText("ok".into()),
            ModelItem::UserText("current draft".into()),
        ],
        "被裁掉轮次区间内的最后一条模型切换必须注入首个保留轮次之前"
    );
    assert!(
        !format!("{:?}", assembly.request.items).contains("Earlier switch"),
        "只保留紧邻首轮的最后一条历史切换"
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
