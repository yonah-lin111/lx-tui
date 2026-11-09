use super::*;
use crate::app::state::{AiWebSite, GrokWebModel, LxEntry};
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
        Some(text::LX_WEB_ALLOWED_CMDS)
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
fn grok_single_stage_model_picker_applies_fast_and_build_without_variants() {
    let (mut state, pane) = ai_web_state();
    crate::app::update::set_ai_web_site(&mut state, pane, AiWebSite::Grok);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target.lx.chat.status.model_name.as_deref(),
        Some("Grok · Fast")
    );
    assert_eq!(target.lx.chat.status.variant, None);

    // 第一级只有 Fast / Build 两项，默认定位 Fast；没有第二级思考面板。
    assert!(super::super::open_model_picker(&mut state, pane));
    {
        let target = state.pane_anywhere(pane).expect("pane");
        let Some(SlashPanel::Model(picker)) = &target.lx.slash else {
            panic!("expected model picker");
        };
        assert_eq!(picker.candidates.len(), 2);
        assert_eq!(picker.current, Some(0));
        assert_eq!(picker.window.active, 0);
    }

    // 选中 Build 回车即完成，不进入空的 Variants 阶段。
    if let Some(target) = state.pane_mut_anywhere(pane)
        && let Some(SlashPanel::Model(picker)) = target.lx.slash.as_mut()
    {
        picker.window.active = 1;
    }
    assert!(super::super::apply_slash_enter(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.slash.is_none());
    assert_eq!(target.lx.ai_web_config.grok_model, GrokWebModel::Build);
    assert_eq!(
        target.lx.chat.status.model_name.as_deref(),
        Some("Grok · Build")
    );
    assert_eq!(target.lx.chat.status.variant, None);
    assert!(
        !state.effects.iter().any(|e| matches!(
            e,
            Effect::SaveDefaultModel { .. } | Effect::SetChatSessionModel { .. }
        )),
        "AI Web 下切换模型不得写回 ai.json 或 SQLite"
    );

    // 发送时透传 Grok 站点、Build 模型与空 thinking。
    draft(&mut state, pane, "Explain quantum computing");
    assert!(super::super::try_send(&mut state, pane));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::StartAiWebStream {
            site: AiWebSite::Grok,
            model,
            thinking,
            prompt,
            ..
        } if model == "Build" && thinking.is_empty() && prompt == "Explain quantum computing"
    )));
}

#[test]
fn grok_rewind_with_attachments_is_rejected_and_keeps_messages() {
    let dir =
        std::env::temp_dir().join(format!("lx-ai-web-grok-rewind-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let doc_path = dir.join("notes.md");
    std::fs::write(&doc_path, "# Notes\nhello\n").expect("write md");

    let (mut state, pane) = ai_web_state();
    crate::app::update::set_ai_web_site(&mut state, pane, AiWebSite::Grok);
    let raw_draft = format!("summarize @[refer-file]({})", doc_path.display());
    draft(&mut state, pane, &raw_draft);
    assert!(super::super::try_send(&mut state, pane));
    finish(&mut state, pane);
    state.effects.clear();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .rewind
        .armed_at = Some(std::time::Instant::now());

    draft(&mut state, pane, "/rewind");
    assert!(super::super::try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.messages.len(), 2, "拒绝后消息不得被截断");
    assert_eq!(target.lx.ai_web_edit_user_index, None);
    assert!(
        target.lx.rewind.armed_at.is_none(),
        "拒绝后必须清除双 Esc 待命状态"
    );
    assert_eq!(
        target.lx.toast.as_ref().map(|t| t.message.as_str()),
        Some(text::LX_AI_WEB_GROK_REWIND_ATTACHMENTS)
    );
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::StartAiWebStream { .. }))
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn grok_text_rewind_restores_prompt_without_truncating_and_next_send_appends() {
    let (mut state, pane) = ai_web_state();
    crate::app::update::set_ai_web_site(&mut state, pane, AiWebSite::Grok);
    draft(&mut state, pane, "first question");
    assert!(super::super::try_send(&mut state, pane));
    finish(&mut state, pane);
    draft(&mut state, pane, "second question");
    assert!(super::super::try_send(&mut state, pane));
    finish(&mut state, pane);
    state.effects.clear();

    draft(&mut state, pane, "/rewind");
    assert!(super::super::try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.messages.len(), 4, "Grok 撤回不截断旧轮次");
    assert_eq!(target.lx.input, "second question");
    assert_eq!(target.lx.ai_web_edit_user_index, None);
    assert_eq!(
        target.lx.toast.as_ref().map(|t| t.message.as_str()),
        Some(text::LX_AI_WEB_GROK_REWIND_RETRY)
    );

    draft(&mut state, pane, "second question edited");
    assert!(super::super::try_send(&mut state, pane));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::StartAiWebStream {
            site: AiWebSite::Grok,
            prompt,
            edit_user_index: None,
            ..
        } if prompt == "second question edited"
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
    assert_eq!(
        ai_web_remote_session("ai-web-grok-abc123"),
        Some((AiWebSite::Grok, "abc123"))
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
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.loading_session = true;
    }
    on_ai_web_session_loaded(
        &mut state,
        pane,
        "ai-web-ds-uuid-1".into(),
        Some("Old Title".into()),
        vec![
            crate::chat::web::AiWebHistoryMessage {
                role: "user".into(),
                thinking: String::new(),
                thinking_ms: None,
                text: "hello".into(),
                files: vec!["photo.png".into(), "report.pdf".into()],
                images: Vec::new(),
                created_at_ms: 1000,
            },
            crate::chat::web::AiWebHistoryMessage {
                role: "assistant".into(),
                thinking: "let me think".into(),
                thinking_ms: Some(1500),
                text: "world".into(),
                files: Vec::new(),
                images: Vec::new(),
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

#[test]
fn stale_ai_web_session_loaded_is_ignored_without_loading_flag() {
    let (mut state, pane) = ai_web_state();
    // 用户已切走（`loading_session` 被清空）后，迟到的云端历史不得覆盖当前窗格或站点。
    let changed = on_ai_web_session_loaded(
        &mut state,
        pane,
        "ai-web-gm-stale".into(),
        Some("Stale".into()),
        Vec::new(),
    );
    assert!(!changed);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.ai_web_site, AiWebSite::Deepseek);
    assert!(target.lx.chat.session_id.is_none());
    assert!(target.lx.chat.messages.is_empty());
}

#[test]
fn ai_web_model_selection_uses_candidate_site_not_pane_site() {
    let (mut state, pane) = ai_web_state();
    crate::app::update::set_ai_web_site(&mut state, pane, AiWebSite::Gemini);

    // 面板在站点切换前构造：候选仍属于 DeepSeek，必须写 DeepSeek 配置而不是 Gemini。
    super::super::slash::finish_model_selection(
        &mut state,
        pane,
        crate::config::ai::ModelSelection {
            provider: text::AI_WEB_SITE_DEEPSEEK.to_string(),
            model: text::AI_WEB_SITE_DEEPSEEK.to_string(),
            variant: Some(text::DEEPSEEK_THINKING_OFF.to_string()),
        },
    );
    let config = state.pane_anywhere(pane).expect("pane").lx.ai_web_config;
    assert_eq!(
        config.deepseek_thinking,
        crate::app::state::DeepseekThinkingMode::Off
    );
    assert_eq!(
        config.gemini_thinking,
        crate::app::state::GeminiThinkingLevel::Low
    );
    assert_eq!(
        config.gemini_model,
        crate::app::state::GeminiWebModel::Flash38
    );
}

#[test]
fn switching_ai_web_site_cancels_in_flight_stream() {
    let (mut state, pane) = ai_web_state();
    draft(&mut state, pane, "hello");
    assert!(super::super::try_send(&mut state, pane));
    state.effects.clear();

    crate::app::update::set_ai_web_site(&mut state, pane, AiWebSite::Gemini);
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::CancelChatStream { .. }))
    );
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.slash.is_none());
    assert!(target.lx.chat.session_id.is_none());
}

#[test]
fn ai_web_generated_images_attach_to_assistant_in_stream_and_resume() {
    let (mut state, pane) = ai_web_state();
    crate::app::update::set_ai_web_site(&mut state, pane, AiWebSite::Grok);
    draft(&mut state, pane, "draw a dog");
    assert!(super::super::try_send(&mut state, pane));
    let key = match &state.pane_anywhere(pane).expect("pane").lx.chat.request {
        RequestState::Streaming { key, .. } => key.clone(),
        other => panic!("expected streaming, got {other:?}"),
    };
    assert!(on_ai_web_image_generated(
        &mut state,
        &key,
        crate::chat::web::AiWebGeneratedImage {
            label: "grok-r1-1.jpg".into(),
            path: Some("/tmp/grok-r1-1.jpg".into()),
            original_url: Some(
                "https://assets.grok.com/users/u1/generated/g1/original_image.jpg".into()
            ),
            preview_url: Some("https://assets.grok.com/users/u1/generated/g1/image.jpg".into()),
            file_stem: Some("grok-94cea5fe-r1-1-orig".into()),
        },
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    let assistant = target.lx.chat.messages.last().expect("assistant");
    assert_eq!(assistant.attachments.len(), 1);
    let att_id = assistant.attachments[0].id.clone();
    assert_eq!(
        assistant.attachments[0].kind,
        crate::db::chat::MessageAttachmentKind::Image
    );
    assert_eq!(assistant.attachments[0].label, "grok-r1-1.jpg");
    assert_eq!(
        assistant.attachments[0].source_path.as_deref(),
        Some("/tmp/grok-r1-1.jpg")
    );
    assert_eq!(
        target
            .lx
            .chat
            .image_originals
            .get(&att_id)
            .map(|o| &o.state),
        Some(&crate::app::state::OriginalImageDownloadState::Idle)
    );

    // 点击下载原图：进入 Downloading 状态并发出 DownloadAiWebOriginalImage 效应，
    // 且经过 Persistence::pump 后不被吞掉，保留给事件层 pump_chat_streams 执行。
    state.effects.clear();
    assert!(super::trigger_ai_web_original_image_download(
        &mut state, pane, &att_id
    ));
    let mut persistence = crate::db::service::Persistence::without_database();
    persistence.pump(&mut state);
    assert!(state.effects.iter().any(|e| matches!(
        e,
        Effect::DownloadAiWebOriginalImage { attachment_id, .. } if attachment_id == &att_id
    )));
    // 下载完成：变为 Downloaded 状态，再次点击则在文件管理器中定位原图（同样不被 Persistence::pump 吞掉）。
    let cur_sid = state
        .pane_anywhere(pane)
        .and_then(|p| p.lx.chat.session_id.clone())
        .expect("sid");
    assert!(super::on_ai_web_original_image_downloaded(
        &mut state,
        pane,
        &cur_sid,
        &att_id,
        "/tmp/Desktop/grok-94cea5fe-r1-1-orig.jpg".into(),
    ));
    state.effects.clear();
    assert!(super::trigger_ai_web_original_image_download(
        &mut state, pane, &att_id
    ));
    persistence.pump(&mut state);
    assert!(state.effects.iter().any(|e| matches!(
        e,
        Effect::RevealInFileManager { path } if path == std::path::Path::new("/tmp/Desktop/grok-94cea5fe-r1-1-orig.jpg")
    )));

    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.loading_session = true;
    }
    assert!(on_ai_web_session_loaded(
        &mut state,
        pane,
        "ai-web-grok-dog".into(),
        Some("Draw a dog".into()),
        vec![
            crate::chat::web::AiWebHistoryMessage {
                role: "user".into(),
                thinking: String::new(),
                thinking_ms: None,
                text: "draw a dog".into(),
                files: Vec::new(),
                images: Vec::new(),
                created_at_ms: 1000,
            },
            crate::chat::web::AiWebHistoryMessage {
                role: "assistant".into(),
                thinking: String::new(),
                thinking_ms: None,
                text: String::new(),
                files: Vec::new(),
                images: vec![crate::chat::web::AiWebGeneratedImage {
                    label: "grok-r1-1.jpg".into(),
                    path: Some("/tmp/grok-r1-1.jpg".into()),
                    original_url: Some(
                        "https://assets.grok.com/users/u1/generated/g1/original_image.jpg".into()
                    ),
                    preview_url: Some(
                        "https://assets.grok.com/users/u1/generated/g1/image.jpg".into()
                    ),
                    file_stem: Some("grok-dog-r1-1-orig".into()),
                }],
                created_at_ms: 2000,
            },
        ],
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.messages.len(), 2);
    assert_eq!(target.lx.chat.messages[1].text(), "Generated image");
    assert_eq!(target.lx.chat.messages[1].attachments.len(), 1);
    assert_eq!(
        target.lx.chat.messages[1].attachments[0]
            .source_path
            .as_deref(),
        Some("/tmp/grok-r1-1.jpg")
    );
}

#[test]
fn original_image_download_receipt_survives_pending_session_binding() {
    let (mut state, pane) = ai_web_state();
    crate::app::update::set_ai_web_site(&mut state, pane, AiWebSite::Gemini);
    draft(&mut state, pane, "draw a cat");
    assert!(super::super::try_send(&mut state, pane));
    let key = match &state.pane_anywhere(pane).expect("pane").lx.chat.request {
        RequestState::Streaming { key, .. } => key.clone(),
        other => panic!("expected streaming, got {other:?}"),
    };
    assert!(on_ai_web_image_generated(
        &mut state,
        &key,
        crate::chat::web::AiWebGeneratedImage {
            label: "gemini-r1-1.png".into(),
            path: Some("/tmp/gemini-r1-1.png".into()),
            original_url: Some("https://lh3.googleusercontent.com/gg/cat=s0-d-I?alr=yes".into()),
            preview_url: Some("https://lh3.googleusercontent.com/gg/cat=w512".into()),
            file_stem: Some("gemini-cat-r1-1-orig".into()),
        },
    ));
    let (att_id, pending_sid) = {
        let target = state.pane_anywhere(pane).expect("pane");
        let att_id = target.lx.chat.messages[1].attachments[0].id.clone();
        let pending_sid = target.lx.chat.session_id.clone().expect("pending session");
        assert!(pending_sid.starts_with("ai-web-"), "{pending_sid}");
        (att_id, pending_sid)
    };

    // 点击 `[Download original]` 时仍处于临时会话，效应冻结该临时 ID。
    state.effects.clear();
    assert!(super::trigger_ai_web_original_image_download(
        &mut state, pane, &att_id
    ));
    let effect_sid = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::DownloadAiWebOriginalImage {
                session_id,
                attachment_id,
                ..
            } if attachment_id == &att_id => Some(session_id.clone()),
            _ => None,
        })
        .expect("download effect");
    assert_eq!(effect_sid, pending_sid);

    // 下载在途时 `SessionBound` 把窗格与原图项升级为云端真实会话 ID。
    assert!(on_ai_web_session_bound(
        &mut state,
        pane,
        &pending_sid,
        "ai-web-gm-real123".into(),
        Some("Cat".into()),
    ));
    {
        let target = state.pane_anywhere(pane).expect("pane");
        assert_eq!(
            target.lx.chat.session_id.as_deref(),
            Some("ai-web-gm-real123")
        );
        assert_eq!(
            target
                .lx
                .chat
                .image_originals
                .get(&att_id)
                .map(|o| o.session_id.as_str()),
            Some("ai-web-gm-real123")
        );
    }

    // 成功回执携带旧临时 ID，仍必须更新按钮为 `[✓ Downloaded]`。
    assert!(super::on_ai_web_original_image_downloaded(
        &mut state,
        pane,
        &pending_sid,
        &att_id,
        "/tmp/Desktop/gemini-cat-r1-1-orig.png".into(),
    ));
    {
        let target = state.pane_anywhere(pane).expect("pane");
        assert!(matches!(
            target.lx.chat.image_originals.get(&att_id).map(|o| &o.state),
            Some(crate::app::state::OriginalImageDownloadState::Downloaded { path })
                if path == "/tmp/Desktop/gemini-cat-r1-1-orig.png"
        ));
        assert!(
            target
                .lx
                .toast
                .as_ref()
                .is_some_and(|t| t.message.contains("Saved original image"))
        );
    }

    // 失败回执携带旧临时 ID 时同样恢复为 `[Download original]` 并提示错误。
    if let Some(orig) = state
        .pane_mut_anywhere(pane)
        .and_then(|p| p.lx.chat.image_originals.get_mut(&att_id))
    {
        orig.state = crate::app::state::OriginalImageDownloadState::Downloading;
    }
    assert!(super::on_ai_web_original_image_download_failed(
        &mut state,
        pane,
        &pending_sid,
        &att_id,
        "network error".into(),
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target
            .lx
            .chat
            .image_originals
            .get(&att_id)
            .map(|o| &o.state),
        Some(&crate::app::state::OriginalImageDownloadState::Idle)
    );
    assert_eq!(
        target.lx.toast.as_ref().map(|t| t.message.as_str()),
        Some("network error")
    );
}

#[test]
fn rewind_drops_original_image_metadata_for_truncated_messages() {
    let (mut state, pane) = ai_web_state();
    draft(&mut state, pane, "draw a cat");
    assert!(super::super::try_send(&mut state, pane));
    let first_key = match &state.pane_anywhere(pane).expect("pane").lx.chat.request {
        RequestState::Streaming { key, .. } => key.clone(),
        other => panic!("expected streaming, got {other:?}"),
    };
    finish(&mut state, pane);
    assert!(on_ai_web_image_generated(
        &mut state,
        &first_key,
        crate::chat::web::AiWebGeneratedImage {
            label: "grok-r1-1.jpg".into(),
            path: Some("/tmp/grok-r1-1.jpg".into()),
            original_url: Some("https://assets.grok.com/u/original_image.jpg".into()),
            preview_url: Some("https://assets.grok.com/u/image.jpg".into()),
            file_stem: Some("grok-cat-r1-1-orig".into()),
        },
    ));

    draft(&mut state, pane, "draw a dog");
    assert!(super::super::try_send(&mut state, pane));
    let second_key = match &state.pane_anywhere(pane).expect("pane").lx.chat.request {
        RequestState::Streaming { key, .. } => key.clone(),
        other => panic!("expected streaming, got {other:?}"),
    };
    finish(&mut state, pane);
    assert!(on_ai_web_image_generated(
        &mut state,
        &second_key,
        crate::chat::web::AiWebGeneratedImage {
            label: "grok-r2-1.jpg".into(),
            path: Some("/tmp/grok-r2-1.jpg".into()),
            original_url: Some("https://assets.grok.com/u/original_image2.jpg".into()),
            preview_url: Some("https://assets.grok.com/u/image2.jpg".into()),
            file_stem: Some("grok-dog-r2-1-orig".into()),
        },
    ));
    let (first_att, second_att) = {
        let target = state.pane_anywhere(pane).expect("pane");
        assert_eq!(target.lx.chat.image_originals.len(), 2);
        (
            target.lx.chat.messages[1].attachments[0].id.clone(),
            target.lx.chat.messages[3].attachments[0].id.clone(),
        )
    };

    draft(&mut state, pane, "/rewind");
    assert!(super::super::try_send(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.messages.len(), 2);
    assert!(
        target.lx.chat.image_originals.contains_key(&first_att),
        "kept message keeps its original image metadata"
    );
    assert!(
        !target.lx.chat.image_originals.contains_key(&second_att),
        "truncated message metadata must be dropped"
    );
}
