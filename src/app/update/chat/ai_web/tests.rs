use super::*;
use crate::app::state::{AiWebSite, LxEntry};
use crate::chat::events::{ChatDelta, PartDelta, StopReason};
use crate::db::chat::Usage;

fn ai_web_state() -> (AppState, PaneId) {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    crate::app::update::set_lx_entry(&mut state, pane, LxEntry::AiWeb);
    (state, pane)
}

fn draft(state: &mut AppState, pane: PaneId, text: &str) {
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.input = text.into();
    }
}

fn finish(state: &mut AppState, pane: PaneId) {
    let key = match &state
        .pane_anywhere(pane)
        .map(|target| &target.lx.chat.request)
    {
        Some(RequestState::Streaming { key, .. }) => key.clone(),
        _ => panic!("expected streaming request"),
    };
    let delta = ChatDelta::Part(PartDelta {
        kind: PartKind::Text,
        text: "reply".into(),
        metadata: None,
    });
    assert!(super::super::on_stream_delta(state, &key, &[delta]));
    assert!(super::super::on_stream_finished(
        state,
        &key,
        &StopReason::Complete,
        Usage::default(),
        Usage::default(),
    ));
}

#[test]
fn send_creates_memory_messages_and_starts_web_stream() {
    let (mut state, pane) = ai_web_state();
    draft(&mut state, pane, "hello");
    assert!(super::super::try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.messages.len(), 2);
    assert_eq!(target.lx.chat.messages[0].role, "user");
    assert_eq!(target.lx.chat.messages[1].role, "assistant");
    assert_eq!(target.lx.chat.messages[1].status, "streaming");
    assert!(matches!(
        target.lx.chat.request,
        RequestState::Streaming { .. }
    ));
    assert_eq!(
        target.lx.chat.status.model_name.as_deref(),
        Some("DeepSeek")
    );
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::StartAiWebStream { .. }))
    );
}

#[test]
fn finished_web_stream_settles_in_memory_without_db_write() {
    let (mut state, pane) = ai_web_state();
    draft(&mut state, pane, "hello");
    assert!(super::super::try_send(&mut state, pane));
    finish(&mut state, pane);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.request, RequestState::Idle);
    let assistant = target.lx.chat.messages.last().expect("assistant");
    assert_eq!(assistant.status, "complete");
    assert_eq!(assistant.text(), "reply");
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::FinishChatTurn { .. }))
    );
}

#[test]
fn new_command_resets_messages_and_web_session() {
    let (mut state, pane) = ai_web_state();
    draft(&mut state, pane, "hello");
    assert!(super::super::try_send(&mut state, pane));
    finish(&mut state, pane);
    draft(&mut state, pane, "/new");
    assert!(super::super::try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.chat.messages.is_empty());
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::ResetAiWebSession {
            site: AiWebSite::Deepseek
        }
    )));
}

#[test]
fn model_command_opens_picker_and_unsupported_commands_are_rejected_in_ai_web() {
    let (mut state, pane) = ai_web_state();
    draft(&mut state, pane, "/model");
    assert!(super::super::try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.chat.messages.is_empty());
    assert!(matches!(target.lx.slash, Some(SlashPanel::Model(_))));

    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.slash = None;
    }
    draft(&mut state, pane, "/model gpt-4o");
    assert!(super::super::try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(
        target.lx.chat.messages.is_empty(),
        "带参本地命令不得被当成普通消息发给网页端"
    );
    assert_eq!(
        target.lx.toast.as_ref().map(|toast| toast.message.as_str()),
        Some(text::LX_AI_WEB_ONLY_NEW_CMD)
    );
}

#[test]
fn gemini_two_stage_model_and_thinking_picker_updates_status_and_stream_without_persisting() {
    let (mut state, pane) = ai_web_state();
    crate::app::update::set_ai_web_site(&mut state, pane, AiWebSite::Gemini);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target.lx.chat.status.model_name.as_deref(),
        Some("Gemini · 3.8 Flash")
    );
    assert_eq!(target.lx.chat.status.variant.as_deref(), Some("low"));

    // 打开 `/model` 第一级：展示 4 个 Gemini 模型，默认定位在 `3.8 Flash` (index 1)。
    assert!(super::super::open_model_picker(&mut state, pane));
    {
        let target = state.pane_anywhere(pane).expect("pane");
        let Some(SlashPanel::Model(picker)) = &target.lx.slash else {
            panic!("expected model picker");
        };
        assert_eq!(picker.candidates.len(), 4);
        assert_eq!(picker.current, Some(1));
        assert_eq!(picker.window.active, 1);
    }

    // 选中第 4 个模型 `4 Argon` (index 3) 并按 Enter 进入第 2 级思考等级。
    if let Some(target) = state.pane_mut_anywhere(pane)
        && let Some(SlashPanel::Model(picker)) = target.lx.slash.as_mut()
    {
        picker.window.active = 3;
    }
    assert!(super::super::apply_slash_enter(&mut state, pane));
    {
        let target = state.pane_anywhere(pane).expect("pane");
        let Some(SlashPanel::Model(picker)) = &target.lx.slash else {
            panic!("expected variants stage");
        };
        let crate::app::state::ModelPickerStage::Variants { model, variants } = &picker.stage
        else {
            panic!("expected Variants stage");
        };
        assert_eq!(model.model, "4 Argon");
        assert_eq!(variants, &vec!["low", "medium", "high"]);
    }

    // 选中 `high` (index 2) 并确认。
    if let Some(target) = state.pane_mut_anywhere(pane)
        && let Some(SlashPanel::Model(picker)) = target.lx.slash.as_mut()
    {
        picker.window.active = 2;
    }
    assert!(super::super::apply_slash_enter(&mut state, pane));

    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.slash.is_none());
    assert_eq!(
        target.lx.chat.status.model_name.as_deref(),
        Some("Gemini · 4 Argon")
    );
    assert_eq!(target.lx.chat.status.variant.as_deref(), Some("high"));
    assert!(
        !state.effects.iter().any(|e| matches!(
            e,
            Effect::SaveDefaultModel { .. } | Effect::SetChatSessionModel { .. }
        )),
        "AI Web 下切换模型不得写回 ai.json 或 SQLite"
    );

    // 发送消息时透传 Gemini 站点、模型 `4 Argon` 与思考等级 `high`。
    draft(&mut state, pane, "Explain quantum computing");
    assert!(super::super::try_send(&mut state, pane));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::StartAiWebStream {
            site: AiWebSite::Gemini,
            model,
            thinking,
            prompt,
            ..
        } if model == "4 Argon" && thinking == "high" && prompt == "Explain quantum computing"
    )));
}

#[test]
fn switching_ai_web_site_isolates_sessions_and_clears_rewind_state() {
    let (mut state, pane) = ai_web_state();
    draft(&mut state, pane, "DeepSeek question");
    assert!(super::super::try_send(&mut state, pane));
    finish(&mut state, pane);
    on_ai_web_session_bound(
        &mut state,
        pane,
        &format!("ai-web-{}-1", pane.raw()),
        "ai-web-ds-123".into(),
        Some("DS Chat".into()),
    );
    draft(&mut state, pane, "/rewind");
    assert!(super::super::try_send(&mut state, pane));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .and_then(|p| p.lx.ai_web_edit_user_index),
        Some(0)
    );

    // 切换到 Gemini 站点：必须清空 DeepSeek 会话与 `/rewind` 待编辑状态。
    crate::app::update::set_ai_web_site(&mut state, pane, AiWebSite::Gemini);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.ai_web_site, AiWebSite::Gemini);
    assert!(target.lx.chat.session_id.is_none());
    assert!(target.lx.chat.messages.is_empty());
    assert!(target.lx.ai_web_edit_user_index.is_none());
    assert_eq!(
        ai_web_remote_session("ai-web-gm-abc123"),
        Some((AiWebSite::Gemini, "abc123"))
    );
    assert_eq!(
        ai_web_remote_session("ai-web-ds-xyz789"),
        Some((AiWebSite::Deepseek, "xyz789"))
    );
}

#[test]
fn multi_turn_send_advances_generation_and_keeps_qa_order() {
    let (mut state, pane) = ai_web_state();
    draft(&mut state, pane, "你是谁");
    assert!(super::super::try_send(&mut state, pane));
    finish(&mut state, pane);

    draft(&mut state, pane, "最近有什么新闻");
    assert!(super::super::try_send(&mut state, pane));
    let key2 = match &state
        .pane_anywhere(pane)
        .map(|target| &target.lx.chat.request)
    {
        Some(RequestState::Streaming { key, .. }) => key.clone(),
        _ => panic!("expected streaming request on turn 2"),
    };
    assert_eq!(key2.generation, 2);
    let delta2 = ChatDelta::Part(PartDelta {
        kind: PartKind::Text,
        text: "second reply".into(),
        metadata: None,
    });
    assert!(super::super::on_stream_delta(&mut state, &key2, &[delta2]));
    assert!(super::super::on_stream_finished(
        &mut state,
        &key2,
        &StopReason::Complete,
        Usage::default(),
        Usage::default(),
    ));

    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.messages.len(), 4);
    assert_eq!(target.lx.chat.messages[0].text(), "你是谁");
    assert_eq!(target.lx.chat.messages[1].text(), "reply");
    assert_eq!(target.lx.chat.messages[2].text(), "最近有什么新闻");
    assert_eq!(target.lx.chat.messages[3].text(), "second reply");
}

#[test]
fn rewind_pops_latest_turn_restores_input_and_marks_edit_index() {
    let (mut state, pane) = ai_web_state();
    draft(&mut state, pane, "第一问");
    assert!(super::super::try_send(&mut state, pane));
    finish(&mut state, pane);

    draft(&mut state, pane, "第二问");
    assert!(super::super::try_send(&mut state, pane));
    finish(&mut state, pane);
    state.effects.clear();

    draft(&mut state, pane, "/rewind");
    assert!(super::super::try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.messages.len(), 2);
    assert_eq!(target.lx.input, "第二问");
    assert_eq!(target.lx.ai_web_edit_user_index, Some(1));

    draft(&mut state, pane, "修改后的第二问");
    assert!(super::super::try_send(&mut state, pane));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::StartAiWebStream {
            prompt,
            edit_user_index: Some(1),
            ..
        } if prompt == "修改后的第二问"
    )));
}

#[test]
fn rename_and_session_load_and_cloud_attachment_rewind_work_in_ai_web() {
    let (mut state, pane) = ai_web_state();
    on_ai_web_session_loaded(
        &mut state,
        pane,
        "ai-web-ds-uuid-1".into(),
        Some("Old Title".into()),
        vec![
            crate::chat::ai_web::AiWebHistoryMessage {
                role: "user".into(),
                thinking: String::new(),
                thinking_ms: None,
                text: "hello".into(),
                files: vec!["photo.png".into(), "report.pdf".into()],
                created_at_ms: 1000,
            },
            crate::chat::ai_web::AiWebHistoryMessage {
                role: "assistant".into(),
                thinking: "let me think".into(),
                thinking_ms: Some(1500),
                text: "world".into(),
                files: Vec::new(),
                created_at_ms: 2000,
            },
        ],
    );
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.messages.len(), 2);
    assert_eq!(target.lx.chat.messages[0].attachments.len(), 2);
    assert_eq!(
        target.lx.chat.messages[0].attachments[0].kind,
        crate::db::chat::MessageAttachmentKind::Image
    );
    assert_eq!(
        target.lx.chat.messages[0].attachments[1].kind,
        crate::db::chat::MessageAttachmentKind::File
    );
    assert_eq!(
        crate::chat::strip_references_for_display(
            &target.lx.chat.messages[0].text(),
            &target.lx.chat.messages[0].attachments
        ),
        "hello"
    );
    assert_eq!(target.lx.chat.session_title.as_deref(), Some("Old Title"));
    assert!(!target.lx.chat.reasoning_expanded(1, 0));

    draft(&mut state, pane, "/rename New Cloud Title");
    assert!(super::super::try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target.lx.chat.session_title.as_deref(),
        Some("New Cloud Title")
    );
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::RenameChatSession {
            session_id,
            title,
            ..
        } if session_id == "ai-web-ds-uuid-1" && title == "New Cloud Title"
    )));

    // Rewind the cloud-loaded turn: restores both cloud attachment markers and clears title when back at home screen
    state.effects.clear();
    draft(&mut state, pane, "/rewind");
    assert!(super::super::try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.chat.messages.is_empty());
    assert!(
        target.lx.chat.session_title.is_none(),
        "撤销全部轮次回到首页后，输入框右上角的会话标题必须清空"
    );
    assert_eq!(
        target.lx.input,
        "hello @[refer-image](photo.png) @[refer-file](report.pdf)"
    );
    assert_eq!(
        target.lx.ai_web_edit_cloud_files,
        vec!["photo.png".to_string(), "report.pdf".to_string()]
    );

    // User deletes photo.png and keeps report.pdf -> keep_files has ["report.pdf"]
    draft(&mut state, pane, "hello @[refer-file](report.pdf)");
    assert!(super::super::try_send(&mut state, pane));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::StartAiWebStream {
            prompt,
            files,
            keep_files,
            edit_user_index: Some(0),
            ..
        } if prompt == "hello" && files.is_empty() && keep_files == &vec!["report.pdf".to_string()]
    )));
}

#[test]
fn file_and_image_upload_and_rewind_attachment_removal_work_in_ai_web() {
    let dir = std::env::temp_dir().join(format!("lx-ai-web-upload-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let img_path = dir.join("diagram.png");
    let doc_path = dir.join("notes.md");
    std::fs::write(
        &img_path,
        [
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I', b'H',
            b'D', b'R',
        ],
    )
    .expect("write png");
    std::fs::write(&doc_path, "# Notes\nhello\n").expect("write md");

    let (mut state, pane) = ai_web_state();
    let raw_draft = format!(
        "请对比这两份材料 @[refer-image]({}) @[refer-file]({})",
        img_path.display(),
        doc_path.display()
    );
    draft(&mut state, pane, &raw_draft);
    assert!(super::super::try_send(&mut state, pane));

    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.messages[0].text(), raw_draft);
    assert_eq!(target.lx.chat.messages[0].attachments.len(), 2);
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::StartAiWebStream {
            prompt,
            files,
            edit_user_index: None,
            ..
        } if prompt == "请对比这两份材料" && files.len() == 2
    )));

    finish(&mut state, pane);
    state.effects.clear();

    draft(&mut state, pane, "/rewind");
    assert!(super::super::try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.input, raw_draft);
    assert_eq!(target.lx.ai_web_edit_user_index, Some(0));

    let edited_draft = format!("只看这份文档 @[refer-file]({})", doc_path.display());
    draft(&mut state, pane, &edited_draft);
    assert!(super::super::try_send(&mut state, pane));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::StartAiWebStream {
            prompt,
            files,
            edit_user_index: Some(0),
            ..
        } if prompt == "只看这份文档" && files.len() == 1
    )));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn folder_reference_is_rejected_in_ai_web_and_keeps_draft() {
    let dir = std::env::temp_dir().join(format!("lx-ai-web-folder-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (mut state, pane) = ai_web_state();
    let raw_draft = format!("看看目录 @[refer-folder]({})", dir.display());
    draft(&mut state, pane, &raw_draft);
    assert!(super::super::try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.chat.messages.is_empty());
    assert_eq!(target.lx.input, raw_draft, "被拦截时保留草稿");
    assert_eq!(
        target.lx.toast.as_ref().map(|t| t.message.as_str()),
        Some(text::LX_AI_WEB_FOLDER_UNSUPPORTED)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ai_web_mention_panel_exposes_only_files_tab_and_confirming_folder_enters_it() {
    let (mut state, pane) = ai_web_state();
    if let Some(target) = state.pane_mut_anywhere(pane) {
        let root = std::path::PathBuf::from("/tmp");
        target.lx.set_mention_root(Some(root.clone()));
        target.lx.insert_char('@');
        let _ = target.lx.take_mention_scan_request();
        target.lx.apply_mention_entries(
            1,
            &root,
            vec![
                crate::app::markdown::MentionEntry {
                    path: "docs".into(),
                    is_directory: true,
                },
                crate::app::markdown::MentionEntry {
                    path: "docs/guide.md".into(),
                    is_directory: false,
                },
            ],
        );
        assert_eq!(
            target.lx.mention_tab(),
            crate::app::prompt::MentionTab::Files
        );
        assert!(
            !target.lx.mention_switch_tab(1),
            "AI Web 禁用 RefDir/Skills/MCP，不可切 Tab"
        );
        assert!(
            !target
                .lx
                .mention_set_tab(crate::app::prompt::MentionTab::RefDir)
        );
        assert!(
            !target
                .lx
                .mention_set_tab(crate::app::prompt::MentionTab::Skills)
        );
        assert!(
            !target
                .lx
                .mention_set_tab(crate::app::prompt::MentionTab::Mcp)
        );

        // Confirming the highlighted "docs" folder in AI Web enters the folder instead of inserting "@docs/ "
        assert!(target.lx.mention_confirm());
        assert_eq!(target.lx.input, "@docs/");
    }
}

#[test]
fn deepseek_model_picker_toggles_deepthink_and_off() {
    let (mut state, pane) = ai_web_state();
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target.lx.chat.status.model_name.as_deref(),
        Some("DeepSeek")
    );
    assert_eq!(target.lx.chat.status.variant.as_deref(), Some("deepthink"));

    assert!(super::super::open_model_picker(&mut state, pane));
    assert!(super::super::apply_slash_enter(&mut state, pane));
    if let Some(target) = state.pane_mut_anywhere(pane)
        && let Some(SlashPanel::Model(picker)) = target.lx.slash.as_mut()
    {
        picker.window.active = 1; // "off"
    }
    assert!(super::super::apply_slash_enter(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.status.variant, None);
}

#[test]
fn switching_between_ai_web_and_native_isolates_sessions_and_blocks_shell_mode() {
    let (mut state, pane) = ai_web_state();
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.insert_char('!');
        assert!(
            !target.lx.shell_mode(),
            "AI Web 下输入 ! 不进入本地 Shell 模式"
        );
        target.lx.clear();
    }
    draft(&mut state, pane, "hello");
    assert!(super::super::try_send(&mut state, pane));
    finish(&mut state, pane);
    draft(&mut state, pane, "/rewind");
    assert!(super::super::try_send(&mut state, pane));

    // Switch back to Native: must clear the ai-web-* session and edit_user_index
    crate::app::update::set_lx_entry(&mut state, pane, LxEntry::Native);
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.chat.session_id.is_none());
    assert!(target.lx.ai_web_edit_user_index.is_none());
}
