use std::collections::BTreeMap;

use super::*;
use crate::app::prompt::PromptMode;
use crate::app::state::ModelsSnapshot;
use crate::config::ai::{
    AiSettings, ModelConfig, ModelSelection, ProviderConfig, ProviderOptions, ProviderTransport,
};

fn sample_models(with_title_model: bool) -> ModelsSnapshot {
    let mut models = BTreeMap::new();
    models.insert(
        "chat-main".to_string(),
        ModelConfig {
            id: "chat-main".to_string(),
            name: "Chat Main".to_string(),
            transport: None,
            limit: None,
            modalities: None,
            variants: BTreeMap::new(),
            variant: None,
            pricing: None,
            extra: serde_json::Map::new(),
        },
    );
    models.insert(
        "title-fast".to_string(),
        ModelConfig {
            id: "title-fast".to_string(),
            name: "Title Fast".to_string(),
            transport: None,
            limit: None,
            modalities: None,
            variants: BTreeMap::new(),
            variant: None,
            pricing: None,
            extra: serde_json::Map::new(),
        },
    );
    let mut providers = BTreeMap::new();
    providers.insert(
        "p".to_string(),
        ProviderConfig {
            id: "p".to_string(),
            kind: ProviderTransport::OpenaiCompatible,
            name: "Provider".to_string(),
            options: ProviderOptions {
                api_key: Some("test-key".to_string()),
                base_url: "https://api.example.invalid/v1".to_string(),
            },
            models,
            extra: serde_json::Map::new(),
        },
    );
    ModelsSnapshot {
        settings: AiSettings {
            enabled_providers: vec!["p".to_string()],
            providers,
            default_model: Some(ModelSelection {
                provider: "p".to_string(),
                model: "chat-main".to_string(),
                variant: None,
            }),
            title_summary: with_title_model.then(|| ModelSelection {
                provider: "p".to_string(),
                model: "title-fast".to_string(),
                variant: None,
            }),
            title_language: None,
            stream_idle_timeout_ms: None,
            web_search: Default::default(),
            extra: serde_json::Map::new(),
        },
    }
}

#[test]
fn first_turn_schedules_session_title_and_manual_rename_cancels_it() {
    let mut state = AppState::demo();
    state.models = sample_models(true);
    let pane = state.active_tab().layout.focus();
    state.pane_mut_anywhere(pane).expect("pane").lx.input = "Implement session title".into();
    crate::app::update::try_send(&mut state, pane);
    let generation = match &state.pane_anywhere(pane).expect("pane").lx.chat.request {
        crate::app::state::RequestState::Committing { generation, .. } => *generation,
        _ => panic!("expected committing"),
    };
    let committed = TurnCommitted {
        session_id: "sess-title-1".into(),
        user_message_id: "u1".into(),
        assistant_message_id: "a1".into(),
        turn_index: 0,
        user_sequence: 0,
        assistant_sequence: 1,
        revision: 1,
        created_at_ms: 1000,
        attachments: Vec::new(),
    };
    assert!(crate::app::update::on_turn_committed(
        &mut state, pane, generation, &committed
    ));
    let title_effect = state.effects.iter().find_map(|effect| match effect {
        Effect::GenerateSessionTitle {
            session_id,
            initial_title,
            model,
            ..
        } => Some((
            session_id.clone(),
            initial_title.clone(),
            model.model.clone(),
        )),
        _ => None,
    });
    assert_eq!(
        title_effect,
        Some((
            "sess-title-1".to_string(),
            "Implement session title".to_string(),
            "title-fast".to_string()
        )),
        "首轮提交应使用配置的 titleSummary 模型入队 GenerateSessionTitle"
    );
    assert!(state.pending_session_titles.contains("sess-title-1"));

    // 手动 /rename 立即作废在途自动标题生成
    assert!(crate::app::update::rename_session(
        &mut state,
        pane,
        "sess-title-1",
        "Manual Name"
    ));
    assert!(
        !take_pending_session_title(&mut state, "sess-title-1"),
        "手动重命名后自动标题已被作废"
    );

    // 自动标题落库回执或手动重命名回执更新窗格顶边框标题胶囊，且同会话再次改名保持配色槽位稳定
    assert!(apply_generated_session_title(
        &mut state,
        "sess-title-1",
        "Auto Title"
    ));
    let palette_first = {
        let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
        assert_eq!(chat.session_title.as_deref(), Some("Auto Title"));
        chat.session_title_palette
    };
    assert!(crate::app::update::on_session_renamed(
        &mut state,
        pane,
        "sess-title-1",
        "Manual Name"
    ));
    {
        let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
        assert_eq!(chat.session_title.as_deref(), Some("Manual Name"));
        assert_eq!(
            chat.session_title_palette, palette_first,
            "同会话改名保持配色槽位稳定不闪烁"
        );
    }

    // /new 解绑会话时清空顶边框标题胶囊，下一次新会话分配不同的配色槽位
    assert!(crate::app::update::new_session(&mut state, pane));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .session_title,
        None
    );
    let next_chat = &mut state.pane_mut_anywhere(pane).expect("pane").lx.chat;
    next_chat.session_id = Some("sess-title-2".into());
    assert!(next_chat.set_session_title(Some("Second Session".into())));
    assert_ne!(
        next_chat.session_title_palette, palette_first,
        "相邻新会话应分配不同的配色槽位"
    );
    assert!(
        next_chat.set_session_title(Some("   \n\t ".into())),
        "空白标题应将原标题清空为 None，不得回退为 Working..."
    );
    assert_eq!(next_chat.session_title, None);
}

#[test]
fn prompt_block_title_handles_rollback_supersede_pages_mode_and_inline_title() {
    let mut state = AppState::demo();
    let initial = "&&& addTemplate --start {title: Old}\n\n- Description: Build feature\n/title\n&&& addTemplate --end";
    state.prompt.set_text(initial);
    let cursor = initial.find("/title").expect("slash") + "/title".len();
    state.prompt.set_cursor_offset(cursor);

    // 1. 未配置任何模型：立即回滚为 Old 并弹 Toast
    assert!(state.prompt.slash_confirm());
    assert!(flush_prompt_title_action(&mut state));
    assert!(
        state.prompt.text().contains("{title: Old}"),
        "无模型时应回滚内联标题为 Old: {}",
        state.prompt.text()
    );
    assert_eq!(
        state.toast.as_ref().map(|t| t.message.as_str()),
        Some(text::PROMPT_TITLE_NO_MODEL)
    );

    // 2. 配置默认模型后（无 titleSummary 回退 defaultModel）：触发阶段不提前发送 SavePromptContent
    state.models = sample_models(false);
    state.effects.clear();
    state.prompt.set_text(initial);
    state.prompt.set_cursor_offset(cursor);
    assert!(state.prompt.slash_confirm());
    assert!(flush_prompt_title_action(&mut state));
    assert!(state.prompt.text().contains("{title: Generating title...}"));
    assert!(
        !state
            .effects
            .iter()
            .any(|e| matches!(e, Effect::SavePromptContent { .. })),
        "触发阶段不得把 Generating title... 占位符提前持久化入库"
    );
    let req1 = state.prompt_title_request_counter;

    // 3. 在同一块再次触发 /title：旧 request_id 被覆盖失效，且保留最初的 Old 作为回滚基准
    let text_with_second_slash = state.prompt.text().replace(
        "- Description: Build feature",
        "- Description: Build feature\n/title",
    );
    state.prompt.set_text(&text_with_second_slash);
    let cursor2 = text_with_second_slash.find("/title").expect("slash2") + "/title".len();
    state.prompt.set_cursor_offset(cursor2);
    assert!(state.prompt.slash_confirm());
    assert!(flush_prompt_title_action(&mut state));
    let req2 = state.prompt_title_request_counter;
    assert_ne!(req1, req2);

    let ws = state.prompt_workspace();
    // 旧请求 req1 迟到返回：被忽略，不覆盖也不报错
    assert!(!on_prompt_block_title_generated(
        &mut state,
        req1,
        ws,
        PromptEditing::Temp(0),
        PromptPageId::Temp(0),
        0,
        Some("Old"),
        Some("Stale Title"),
    ));

    // 切换到 Pages 列表视图期间新请求 req2 完成：同步更新 state.prompt 缓冲
    state.workspaces[ws].prompt_panel.mode = PromptMode::Pages;
    assert!(on_prompt_block_title_generated(
        &mut state,
        req2,
        ws,
        PromptEditing::Temp(0),
        PromptPageId::Temp(0),
        0,
        None,
        Some("Build feature title"),
    ));
    assert!(
        state.prompt.text().contains("{title: Build feature title}"),
        "Pages 模式下完成回填也应同步更新编辑器缓冲: {}",
        state.prompt.text()
    );
}
