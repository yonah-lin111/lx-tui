use super::*;

#[test]
fn prompt_clamps_within_byte_limit_on_utf8_boundary_and_stays_english() {
    let long_cjk = "修复设置页模型与供应商拆分以及自动生成标题问题".repeat(40);
    let prompt = session_title_prompt(&long_cjk).expect("prompt");
    assert!(prompt.len() <= TITLE_PROMPT_MAX_BYTES);
    assert!(
        prompt.contains("Simplified Chinese"),
        "默认 Auto 检测到中文输入应要求输出简体中文"
    );
    assert!(prompt.contains("Conversation content:"));

    // 校验所有语言分支的系统指令本身为纯 ASCII 英文（遵守模型提示词全英文规范）
    for lang in [
        TitleLanguage::Auto,
        TitleLanguage::SimplifiedChinese,
        TitleLanguage::TraditionalChinese,
        TitleLanguage::English,
        TitleLanguage::Japanese,
        TitleLanguage::Korean,
    ] {
        let instr = title_instructions_for(lang, "test");
        assert!(
            instr.is_ascii(),
            "title_instructions_for({lang:?}) 必须为纯英文 ASCII: {instr}"
        );
    }

    let tpl_prompt =
        template_block_title_prompt_for(&long_cjk, TitleLanguage::English).expect("en prompt");
    assert!(tpl_prompt.len() <= TITLE_PROMPT_MAX_BYTES);
    assert!(tpl_prompt.contains("You MUST write the title in English"));
    assert!(tpl_prompt.contains("Task template content:"));
}

#[test]
fn auto_language_detects_cjk_traditional_kana_and_ignores_middle_dot() {
    let cjk_prompt =
        template_block_title_prompt_for("- Description: 优化标题生成语言", TitleLanguage::Auto)
            .expect("auto cjk");
    assert!(cjk_prompt.contains("Simplified Chinese"));

    // 含中点 `・` (U+30FB) 的简体中文不得误判为日文
    let middle_dot_cjk =
        session_title_prompt_for("修复卡尔・马克思引用展示问题", TitleLanguage::Auto)
            .expect("middle dot cjk");
    assert!(middle_dot_cjk.contains("Simplified Chinese"));

    // 繁体中文输入应识别为 Traditional Chinese
    let trad_prompt =
        session_title_prompt_for("優化設定頁標題產生與歷史紀錄顯示", TitleLanguage::Auto)
            .expect("trad cjk");
    assert!(trad_prompt.contains("Traditional Chinese"));

    let en_prompt =
        template_block_title_prompt_for("- Description: Fix memory leak", TitleLanguage::Auto)
            .expect("auto non-cjk");
    assert!(en_prompt.contains("Ignore English template field labels"));
}

#[test]
fn empty_input_returns_none() {
    assert!(session_title_prompt("   \n\t ").is_none());
    assert!(template_block_title_prompt("").is_none());
    assert!(parse_generated_title("   ").is_none());
    assert!(parse_generated_title("<think>only thoughts</think>").is_none());
}

#[test]
fn parse_generated_title_rejects_refusal_phrases_without_false_positive_on_valid_tasks() {
    assert!(parse_generated_title("无法生成任务标题：输入不是任务描述").is_none());
    assert!(parse_generated_title("不能生成标题：未提供任务描述").is_none());
    assert!(parse_generated_title("無法生成任務標題：輸入不是任務描述").is_none());
    assert!(parse_generated_title("タイトルを生成できません").is_none());
    assert!(parse_generated_title("제목을 생성할 수 없습니다").is_none());
    assert!(parse_generated_title("Cannot generate title: input is not a task").is_none());

    // 正常包含「无法生成」的技术问题标题不得被误杀
    assert_eq!(
        parse_generated_title("修复无法生成 PDF 报表问题").as_deref(),
        Some("修复无法生成 PDF 报表问题")
    );
}

#[test]
fn parse_generated_title_handles_think_tags_fenced_json_label_prefix_and_punctuation() {
    let raw = "<think>\nLet me summarize this.\n</think>\n\"# Title: Fix login token refresh!\"";
    assert_eq!(
        parse_generated_title(raw).as_deref(),
        Some("Fix login token refresh")
    );

    let fenced_json = "```json\n{\"title\": \"重构设置页 Model 与 Provider 标签。\"}\n```";
    assert_eq!(
        parse_generated_title(fenced_json).as_deref(),
        Some("重构设置页 Model 与 Provider 标签")
    );
    assert!(parse_generated_title("{\"title\": null}").is_none());
}

#[test]
fn parse_generated_title_strips_braces_and_truncates_to_36_chars() {
    let raw = "Implement {title: test} with a very long title exceeding thirty six characters";
    let parsed = parse_generated_title(raw).expect("parsed");
    assert!(!parsed.contains('{') && !parsed.contains('}'));
    assert!(parsed.chars().count() <= TITLE_MAX_CHARS);
}
