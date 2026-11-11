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
    assert!(tpl_prompt.len() <= TEMPLATE_TITLE_PROMPT_MAX_BYTES);
    assert!(tpl_prompt.len() > TITLE_PROMPT_MAX_BYTES);
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

#[test]
fn template_prompt_contains_db_style_examples_and_session_prompt_does_not() {
    let zh = template_block_title_prompt_for(
        "- Description: 修改 todo item 的缩进",
        TitleLanguage::Auto,
    )
    .expect("zh prompt");
    assert!(zh.contains("修改 todo item 的缩进"), "{zh}");
    assert!(zh.contains("添加 /mcp 命令"), "{zh}");
    assert!(zh.contains("为 prompt 输入框添加省略"), "{zh}");
    assert!(zh.contains("Never output a generic verb alone"), "{zh}");

    let en = template_block_title_prompt_for("- Description: Fix indentation", TitleLanguage::Auto)
        .expect("en prompt");
    assert!(en.contains("Fix todo item indentation"), "{en}");
    assert!(en.contains("Truncate long prompt input"), "{en}");
    assert!(en.contains("Never output a generic verb alone"), "{en}");

    let session = session_title_prompt_for("Fix indentation", TitleLanguage::English)
        .expect("session prompt");
    assert!(!session.contains("Examples of good titles"), "{session}");
    assert!(!session.contains("Fix todo item indentation"), "{session}");
    assert!(
        !session.contains("Never output a generic verb alone"),
        "{session}"
    );
}

#[test]
fn template_prompt_clamps_to_2048_and_keeps_utf8_boundary() {
    let long_cjk = "修复设置页模型与供应商拆分以及自动生成标题问题".repeat(200);
    let prompt = template_block_title_prompt_for(&long_cjk, TitleLanguage::SimplifiedChinese)
        .expect("template prompt");
    assert!(
        prompt.len() <= TEMPLATE_TITLE_PROMPT_MAX_BYTES,
        "{}",
        prompt.len()
    );
    assert!(prompt.contains("Task template content:"));
    assert!(!prompt.contains('\u{FFFD}'));
    assert_eq!(
        prompt.len(),
        prompt.trim_end().len(),
        "截断后不得留下半截字符"
    );

    let session = session_title_prompt_for(&long_cjk, TitleLanguage::SimplifiedChinese)
        .expect("session prompt");
    assert!(session.len() <= TITLE_PROMPT_MAX_BYTES);
    assert!(!session.contains("Examples of good titles"));
}

#[test]
fn template_title_instructions_follow_language_branches() {
    let zh = template_title_instructions_for(TitleLanguage::SimplifiedChinese, "修改缩进");
    assert!(zh.contains("Simplified Chinese"), "{zh}");
    assert!(zh.contains("修改 todo item 的缩进"), "{zh}");

    let trad = template_title_instructions_for(TitleLanguage::TraditionalChinese, "修改縮排");
    assert!(trad.contains("Traditional Chinese"), "{trad}");
    assert!(!trad.contains("修改 todo item 的缩进"), "{trad}");

    let ja = template_title_instructions_for(TitleLanguage::Japanese, "タイトル修正");
    assert!(ja.contains("Japanese"), "{ja}");

    let ko = template_title_instructions_for(TitleLanguage::Korean, "제목 수정");
    assert!(ko.contains("Korean"), "{ko}");

    let en = template_title_instructions_for(TitleLanguage::English, "Fix indentation");
    assert!(en.contains("English"), "{en}");
    assert!(en.contains("Fix todo item indentation"), "{en}");

    let auto_en = template_title_instructions_for(TitleLanguage::Auto, "Fix indentation");
    assert!(
        auto_en.contains("Ignore English template field labels"),
        "{auto_en}"
    );
    assert!(auto_en.contains("Fix todo item indentation"), "{auto_en}");

    let auto_zh = template_title_instructions_for(TitleLanguage::Auto, "修改缩进");
    assert!(auto_zh.contains("Simplified Chinese"), "{auto_zh}");
    assert!(auto_zh.contains("修改 todo item 的缩进"), "{auto_zh}");
}

#[test]
fn template_prompt_specifies_six_to_twenty_chinese_chars() {
    let zh = template_title_instructions_for(TitleLanguage::SimplifiedChinese, "修改缩进");
    assert!(
        zh.contains("6 to 20 Chinese characters"),
        "模板标题必须显式约束 6–20 个汉字: {zh}"
    );
    let session = session_title_prompt_for("修改缩进", TitleLanguage::SimplifiedChinese)
        .expect("session prompt");
    assert!(
        !session.contains("6 to 20 Chinese characters"),
        "会话标题预算/约束不得被模板指令改变: {session}"
    );
}

#[test]
fn template_prompt_auto_en_specifies_five_words() {
    let en = template_title_instructions_for(TitleLanguage::Auto, "Fix indentation");
    assert!(
        en.contains("at most 36 characters and at most 5 words"),
        "{en}"
    );
    assert!(!en.contains("Add scrollbar to task panel"), "{en}");
    assert!(en.contains("Add task panel scrollbar"), "{en}");
    assert!(en.contains("Return exactly one title, not a list."), "{en}");
}

#[test]
fn sanitize_title_strips_mixed_prefixes_and_emphasis() {
    for (raw, expected) in [
        ("Title: - Fix bug", "Fix bug"),
        ("- # Fix bug", "Fix bug"),
        ("**Fix bug**", "Fix bug"),
        ("_Fix bug_", "Fix bug"),
        ("\"Fix bug\".", "Fix bug"),
        ("{title: Fix bug}", "Fix bug"),
        ("Fix bug…", "Fix bug"),
        ("Fix bug，", "Fix bug"),
        ("Fix bug）", "Fix bug"),
    ] {
        assert_eq!(
            parse_generated_title(raw).as_deref(),
            Some(expected),
            "raw={raw:?}"
        );
    }
}

#[test]
fn parse_generated_title_keeps_legitimate_title_generation_task() {
    assert_eq!(
        parse_generated_title("修复无法生成标题的 bug").as_deref(),
        Some("修复无法生成标题的 bug")
    );
}

#[test]
fn parse_generated_title_strips_leading_list_markers() {
    for (raw, expected) in [
        ("- Fix todo indentation", "Fix todo indentation"),
        ("* Add /mcp command", "Add /mcp command"),
        ("+ Add scrollbar", "Add scrollbar"),
        (
            "1. Truncate long prompt input",
            "Truncate long prompt input",
        ),
        ("12. 修改 todo item 的缩进", "修改 todo item 的缩进"),
    ] {
        assert_eq!(parse_generated_title(raw).as_deref(), Some(expected));
    }
}
