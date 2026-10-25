//! `ask` 参数校验与结果构造的纯函数测试。

use super::*;

/// 构造合法参数（单题、两选项）。
fn valid_json() -> String {
    serde_json::json!({
        "questions": [{
            "id": "layout",
            "header": "布局",
            "question": "侧栏用哪种布局？",
            "options": [
                {"label": "双栏", "description": "左侧列表、右侧正文。"},
                {"label": "单栏", "description": "全宽正文，切换靠快捷键。"}
            ]
        }]
    })
    .to_string()
}

/// 解析并断言成功。
fn parse_ok(json: &str) -> Vec<AskQuestion> {
    parse_arguments(json).expect("arguments must parse")
}

/// 解析并断言失败（返回错误文案）。
fn parse_err(json: &str) -> String {
    parse_arguments(json)
        .expect_err("arguments must be rejected")
        .message
}

/// 构造恰好 `bytes` 字节、且行数/单行长度都在上限内的 preview。
fn preview_of_bytes(bytes: usize) -> String {
    let line = "x".repeat(MAX_ASK_PREVIEW_LINE_CHARS);
    let mut preview = String::new();
    while preview.len() < bytes {
        if !preview.is_empty() {
            preview.push('\n');
        }
        let room = bytes.saturating_sub(preview.len());
        let take = room.min(MAX_ASK_PREVIEW_LINE_CHARS);
        preview.push_str(&line[..take]);
        // 剩余不足一行时补完即结束。
        if take < MAX_ASK_PREVIEW_LINE_CHARS {
            break;
        }
    }
    preview
}

/// 单题 JSON：可指定题号相关字段。
fn question_json(id: &str, header: &str, question: &str, options: serde_json::Value) -> String {
    serde_json::json!({
        "questions": [{
            "id": id,
            "header": header,
            "question": question,
            "options": options,
        }]
    })
    .to_string()
}

// ===== 合法解析 =====

#[test]
fn parses_a_single_question_with_options() {
    let questions = parse_ok(&valid_json());
    assert_eq!(questions.len(), 1);
    let question = &questions[0];
    assert_eq!(question.id, "layout");
    assert_eq!(question.header, "布局");
    assert!(!question.multi_select);
    assert_eq!(question.options.len(), 2);
    assert_eq!(question.options[0].label, "双栏");
    assert_eq!(question.options[0].description, "左侧列表、右侧正文。");
    assert_eq!(question.options[0].preview, None);
}

#[test]
fn options_may_be_absent_or_empty_for_free_form_questions() {
    let absent = r#"{"questions":[{"id":"q","header":"问","question":"说说看？"}]}"#;
    assert!(parse_ok(absent)[0].options.is_empty());
    let empty = question_json("q", "问", "说说看？", serde_json::json!([]));
    assert!(parse_ok(&empty)[0].options.is_empty());
}

#[test]
fn accepts_one_and_four_options_and_three_questions() {
    for count in [1, 4] {
        let options: Vec<serde_json::Value> = (0..count)
            .map(
                |index| serde_json::json!({"label": format!("选项{index}"), "description": "说明"}),
            )
            .collect();
        let json = question_json("q", "问", "选哪个？", serde_json::json!(options));
        assert_eq!(parse_ok(&json)[0].options.len(), count);
    }
    let questions: Vec<serde_json::Value> = (0..MAX_ASK_QUESTIONS)
        .map(|index| {
            serde_json::json!({
                "id": format!("q{index}"),
                "header": "问",
                "question": "选哪个？",
            })
        })
        .collect();
    let json = serde_json::json!({"questions": questions}).to_string();
    assert_eq!(parse_ok(&json).len(), MAX_ASK_QUESTIONS);
}

#[test]
fn multi_select_and_preview_are_preserved_verbatim() {
    let preview = "┌──┐\n│  │\n└──┘";
    let json = serde_json::json!({
        "questions": [{
            "id": "q",
            "header": "问",
            "question": "选哪个？",
            "multi_select": true,
            "options": [{"label": "A", "description": "说明", "preview": preview}]
        }]
    })
    .to_string();
    let question = &parse_ok(&json)[0];
    assert!(question.multi_select);
    assert_eq!(question.options[0].preview.as_deref(), Some(preview));
}

#[test]
fn tabs_and_newlines_are_accepted_but_other_control_characters_are_not() {
    let allowed = question_json("q", "问", "第一行\n第二行\t制表", serde_json::json!([]));
    assert!(parse_ok(&allowed)[0].question.contains('\t'));

    let rejected = question_json("q", "问", "有\u{7}铃声", serde_json::json!([]));
    let message = parse_err(&rejected);
    assert!(message.contains("control character"), "{message}");
}

#[test]
fn wide_characters_fit_the_character_limits() {
    // 12 个宽字符恰好等于 header 上限（按字符而非字节计）。
    let header: String = "宽".repeat(MAX_ASK_HEADER_CHARS);
    let json = question_json("q", &header, "问", serde_json::json!([]));
    assert_eq!(
        parse_ok(&json)[0].header.chars().count(),
        MAX_ASK_HEADER_CHARS
    );
}

// ===== 拒绝规则 =====

#[test]
fn rejects_non_object_and_missing_questions() {
    assert!(parse_err("[]").contains("must be a JSON object"));
    assert!(parse_err("{}").contains("missing required argument 'questions'"));
    assert!(parse_err("{not json").contains("not valid JSON"));
    assert!(parse_err("   ").contains("must be a JSON object"));
}

#[test]
fn rejects_unknown_top_level_field() {
    let json = serde_json::json!({"questions": [], "extra": 1}).to_string();
    let message = parse_err(&json);
    assert!(message.contains("unknown argument 'extra'"), "{message}");
}

#[test]
fn rejects_empty_and_oversized_question_lists() {
    assert!(parse_err(r#"{"questions":[]}"#).contains("ask requires 1 to 3 questions"));

    let questions: Vec<serde_json::Value> = (0..=MAX_ASK_QUESTIONS)
        .map(|index| {
            serde_json::json!({"id": format!("q{index}"), "header": "问", "question": "问"})
        })
        .collect();
    let json = serde_json::json!({"questions": questions}).to_string();
    assert!(parse_err(&json).contains("ask requires 1 to 3 questions"));
}

#[test]
fn rejects_non_array_questions_and_non_object_entries() {
    assert!(parse_err(r#"{"questions":"x"}"#).contains("'questions' must be an array"));
    let json = serde_json::json!({"questions": ["x"]}).to_string();
    assert!(parse_err(&json).contains("question 1 must be an object"));
}

#[test]
fn rejects_missing_required_question_fields() {
    for field in ["id", "header", "question"] {
        let mut entry = serde_json::json!({
            "id": "q",
            "header": "问",
            "question": "问？",
        });
        entry.as_object_mut().expect("object").remove(field);
        let json = serde_json::json!({"questions": [entry]}).to_string();
        let message = parse_err(&json);
        assert!(
            message.contains(&format!("missing required field \"{field}\"")),
            "{message}"
        );
    }
}

#[test]
fn rejects_wrong_types_and_unknown_question_fields() {
    let json = serde_json::json!({"questions": [{"id": 1, "header": "问", "question": "问？"}]})
        .to_string();
    assert!(parse_err(&json).contains("\"id\" must be a string"));

    let json = serde_json::json!({
        "questions": [{"id": "q", "header": "问", "question": "问？", "extra": true}]
    })
    .to_string();
    assert!(parse_err(&json).contains("unknown field 'extra'"));

    let json = serde_json::json!({
        "questions": [{"id": "q", "header": "问", "question": "问？", "multi_select": "yes"}]
    })
    .to_string();
    assert!(parse_err(&json).contains("\"multi_select\" must be a boolean"));
}

#[test]
fn rejects_bad_option_shapes() {
    let json = serde_json::json!({
        "questions": [{"id": "q", "header": "问", "question": "问？", "options": "x"}]
    })
    .to_string();
    assert!(parse_err(&json).contains("\"options\" must be an array"));

    let json = serde_json::json!({
        "questions": [{"id": "q", "header": "问", "question": "问？", "options": [1]}]
    })
    .to_string();
    assert!(parse_err(&json).contains("option 1 must be an object"));

    let json = serde_json::json!({
        "questions": [{"id": "q", "header": "问", "question": "问？",
                       "options": [{"label": "A"}]}]
    })
    .to_string();
    assert!(parse_err(&json).contains("missing required field \"description\""));

    let json = serde_json::json!({
        "questions": [{"id": "q", "header": "问", "question": "问？",
                       "options": [{"label": "A", "description": "d", "extra": 1}]}]
    })
    .to_string();
    assert!(parse_err(&json).contains("unknown field 'extra'"));

    let preview_not_string = serde_json::json!({
        "questions": [{"id": "q", "header": "问", "question": "问？",
                       "options": [{"label": "A", "description": "d", "preview": 1}]}]
    })
    .to_string();
    assert!(parse_err(&preview_not_string).contains("\"preview\" must be a string"));
}

#[test]
fn rejects_more_than_four_options() {
    let options: Vec<serde_json::Value> = (0..=MAX_ASK_OPTIONS)
        .map(|index| serde_json::json!({"label": format!("O{index}"), "description": "d"}))
        .collect();
    let json = question_json("q", "问", "问？", serde_json::json!(options));
    assert!(parse_err(&json).contains("at most 4 options"));
}

#[test]
fn rejects_duplicate_and_malformed_ids() {
    let json = serde_json::json!({
        "questions": [
            {"id": "same", "header": "一", "question": "问？"},
            {"id": "same", "header": "二", "question": "问？"}
        ]
    })
    .to_string();
    assert!(parse_err(&json).contains("duplicate question id \"same\""));

    for bad in ["Q", "with-dash", "空格", ""] {
        let json = question_json(bad, "问", "问？", serde_json::json!([]));
        let message = parse_err(&json);
        assert!(
            message.contains("\"id\" must not be empty")
                || message.contains("lowercase letters, digits, and underscores"),
            "{bad}: {message}"
        );
    }
}

#[test]
fn rejects_duplicate_option_labels_case_sensitively() {
    let json = serde_json::json!({
        "questions": [{"id": "q", "header": "问", "question": "问？",
            "options": [
                {"label": "A", "description": "d"},
                {"label": "A", "description": "d"}
            ]}]
    })
    .to_string();
    assert!(parse_err(&json).contains("duplicate option label \"A\""));

    // 大小写敏感：`A` 与 `a` 是两个不同标签。
    let json = serde_json::json!({
        "questions": [{"id": "q", "header": "问", "question": "问？",
            "options": [
                {"label": "A", "description": "d"},
                {"label": "a", "description": "d"}
            ]}]
    })
    .to_string();
    assert_eq!(parse_ok(&json)[0].options.len(), 2);
}

#[test]
fn character_limits_are_enforced_at_the_boundary() {
    let exact_header: String = "h".repeat(MAX_ASK_HEADER_CHARS);
    let over_header: String = "h".repeat(MAX_ASK_HEADER_CHARS + 1);
    let json = question_json("q", &exact_header, "问？", serde_json::json!([]));
    parse_ok(&json);
    let json = question_json("q", &over_header, "问？", serde_json::json!([]));
    let message = parse_err(&json);
    assert!(
        message.contains("\"header\" is longer than 12 characters"),
        "{message}"
    );

    let exact_question: String = "q".repeat(MAX_ASK_QUESTION_CHARS);
    let over_question: String = "q".repeat(MAX_ASK_QUESTION_CHARS + 1);
    let json = question_json("q", "问", &exact_question, serde_json::json!([]));
    parse_ok(&json);
    let json = question_json("q", "问", &over_question, serde_json::json!([]));
    assert!(parse_err(&json).contains("\"question\" is longer than 500 characters"));

    let exact_label: String = "l".repeat(MAX_ASK_LABEL_CHARS);
    let over_label: String = "l".repeat(MAX_ASK_LABEL_CHARS + 1);
    let options = serde_json::json!([{"label": exact_label, "description": "d"}]);
    let json = question_json("q", "问", "问？", options);
    parse_ok(&json);
    let options = serde_json::json!([{"label": over_label, "description": "d"}]);
    let json = question_json("q", "问", "问？", options);
    assert!(parse_err(&json).contains("\"label\" is longer than 80 characters"));

    let exact_desc: String = "d".repeat(MAX_ASK_DESC_CHARS);
    let over_desc: String = "d".repeat(MAX_ASK_DESC_CHARS + 1);
    let options = serde_json::json!([{"label": "A", "description": exact_desc}]);
    let json = question_json("q", "问", "问？", options);
    parse_ok(&json);
    let options = serde_json::json!([{"label": "A", "description": over_desc}]);
    let json = question_json("q", "问", "问？", options);
    assert!(parse_err(&json).contains("\"description\" is longer than 200 characters"));

    let exact_id: String = "i".repeat(MAX_ASK_ID_CHARS);
    let over_id: String = "i".repeat(MAX_ASK_ID_CHARS + 1);
    let json = question_json(&exact_id, "问", "问？", serde_json::json!([]));
    parse_ok(&json);
    let json = question_json(&over_id, "问", "问？", serde_json::json!([]));
    assert!(parse_err(&json).contains("\"id\" is longer than 64 characters"));
}

#[test]
fn preview_limits_are_enforced() {
    // 恰好 4096 字节合法，4097 拒绝（行数与单行长度都在限额内）。
    let exact = preview_of_bytes(MAX_ASK_PREVIEW_BYTES);
    assert_eq!(exact.len(), MAX_ASK_PREVIEW_BYTES);
    let options = serde_json::json!([{"label": "A", "description": "d", "preview": exact}]);
    parse_ok(&question_json("q", "问", "问？", options));
    let over = preview_of_bytes(MAX_ASK_PREVIEW_BYTES + 1);
    let options = serde_json::json!([{"label": "A", "description": "d", "preview": over}]);
    let message = parse_err(&question_json("q", "问", "问？", options));
    assert!(message.contains("preview exceeds 4096 bytes"), "{message}");

    // 行数：40 行合法，41 行拒绝。
    let exact_lines = vec!["x"; MAX_ASK_PREVIEW_LINES].join("\n");
    let options = serde_json::json!([{"label": "A", "description": "d", "preview": exact_lines}]);
    parse_ok(&question_json("q", "问", "问？", options));
    let over_lines = vec!["x"; MAX_ASK_PREVIEW_LINES + 1].join("\n");
    let options = serde_json::json!([{"label": "A", "description": "d", "preview": over_lines}]);
    let message = parse_err(&question_json("q", "问", "问？", options));
    assert!(message.contains("more than 40 lines"), "{message}");

    // 单行字符：400 合法，401 拒绝。
    let exact_line = "x".repeat(MAX_ASK_PREVIEW_LINE_CHARS);
    let options = serde_json::json!([{"label": "A", "description": "d", "preview": exact_line}]);
    parse_ok(&question_json("q", "问", "问？", options));
    let over_line = "x".repeat(MAX_ASK_PREVIEW_LINE_CHARS + 1);
    let options = serde_json::json!([{"label": "A", "description": "d", "preview": over_line}]);
    let message = parse_err(&question_json("q", "问", "问？", options));
    assert!(
        message.contains("preview line 1 is longer than 400 characters"),
        "{message}"
    );
}

#[test]
fn preview_total_bytes_are_capped_across_the_whole_call() {
    // 每题 4 个选项、每条 preview 约 3 KiB：单条合法，合计超 24 KiB。
    let preview = vec!["x".repeat(MAX_ASK_PREVIEW_LINE_CHARS); 8].join("\n");
    let options: Vec<serde_json::Value> = (0..MAX_ASK_OPTIONS)
        .map(|index| {
            serde_json::json!({"label": format!("O{index}"), "description": "d", "preview": preview})
        })
        .collect();
    let questions: Vec<serde_json::Value> = (0..MAX_ASK_QUESTIONS)
        .map(|index| {
            serde_json::json!({
                "id": format!("q{index}"),
                "header": "问",
                "question": "问？",
                "options": options,
            })
        })
        .collect();
    let json = serde_json::json!({"questions": questions}).to_string();
    let message = parse_err(&json);
    assert!(message.contains("24576 bytes in total"), "{message}");
}

#[test]
fn oversized_question_text_is_rejected_before_scanning_characters() {
    // 1 MiB 输入：长度检查立即拒绝（不逐字符扫描），并给出稳定文案。
    let huge = "q".repeat(1024 * 1024);
    let json = question_json("q", "问", &huge, serde_json::json!([]));
    let message = parse_err(&json);
    assert!(message.contains("\"question\" is longer than 500 characters"));
}

// ===== answered_result =====

fn question(id: &str, header: &str) -> AskQuestion {
    AskQuestion {
        id: id.to_string(),
        header: header.to_string(),
        question: format!("{header}的问题？"),
        multi_select: false,
        options: Vec::new(),
    }
}

#[test]
fn answered_result_follows_question_order() {
    let questions = vec![question("a", "一"), question("b", "二")];
    let mut answers = AskAnswers::new();
    answers.insert("b".into(), vec!["B".into()]);
    answers.insert("a".into(), vec!["A".into()]);
    let ToolExecutionResult::Ask { questions } = answered_result(&questions, &answers) else {
        panic!("expected ask result");
    };
    assert_eq!(
        questions.iter().map(|q| q.id.as_str()).collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert_eq!(questions[0].answers, vec!["A"]);
    assert_eq!(questions[1].answers, vec!["B"]);
    assert_eq!(questions[0].header, "一");
    assert_eq!(questions[0].question, "一的问题？");
}

#[test]
fn answered_result_dedupes_keeping_first_occurrence() {
    let questions = vec![question("a", "一")];
    let mut answers = AskAnswers::new();
    answers.insert("a".into(), vec!["A".into(), "B".into(), "A".into()]);
    let ToolExecutionResult::Ask { questions } = answered_result(&questions, &answers) else {
        panic!("expected ask result");
    };
    assert_eq!(questions[0].answers, vec!["A", "B"]);
}

#[test]
fn answered_result_keeps_custom_text_last() {
    let questions = vec![question("a", "一")];
    let mut answers = AskAnswers::new();
    // 客户端约定：勾选项在前，手写文本在最后。
    answers.insert(
        "a".into(),
        vec!["A".into(), "自己写一版".into(), "B".into()],
    );
    let ToolExecutionResult::Ask { questions } = answered_result(&questions, &answers) else {
        panic!("expected ask result");
    };
    assert_eq!(questions[0].answers.last().map(String::as_str), Some("B"));
    assert_eq!(
        questions[0].answers.len(),
        3,
        "手写文本原样保留，不参与重排"
    );
}

#[test]
fn answered_result_missing_id_is_empty_not_fabricated() {
    let questions = vec![question("a", "一")];
    let ToolExecutionResult::Ask { questions } = answered_result(&questions, &AskAnswers::new())
    else {
        panic!("expected ask result");
    };
    assert!(questions[0].answers.is_empty());
}
