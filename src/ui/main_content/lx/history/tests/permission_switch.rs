//! 权限切换提示卡片（permission switch notice）渲染、连续合并与点击折叠测试。

use super::*;
use crate::app::state::PermissionSwitchNoticeView;

fn perm_switch(sequence: i64, from: &str, to: &str, prompt: &str) -> PermissionSwitchNoticeView {
    PermissionSwitchNoticeView {
        id: format!("psw-{sequence}"),
        sequence,
        from_mode: from.into(),
        to_mode: to.into(),
        injected_prompt: prompt.into(),
        created_at_ms: 1_761_300_000_000 + sequence * 1_000,
    }
}

fn rows_text(grid: &Grid, area: Rect) -> Vec<String> {
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| grid.cell(x, y).ch)
                .collect::<String>()
        })
        .collect()
}

#[test]
fn permission_switch_coalesces_chain_and_toggles_prompt_preview() {
    let mut lx = Lx::new();
    lx.chat.messages = vec![ChatMessageView {
        id: "u1".into(),
        sequence: 0,
        turn_index: 0,
        role: "user".into(),
        status: "complete".into(),
        kind: "chat".into(),
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: "hello".into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        reports: Vec::new(),
        attachments: Vec::new(),
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    }];
    lx.permission_switch.replace(vec![
        perm_switch(
            1,
            "read-only",
            "workspace-write",
            "Step 1\nline 2\nline 3\nline 4\nline 5",
        ),
        perm_switch(
            2,
            "workspace-write",
            "danger-full-access",
            "Step 2\nline 2\nline 3\nline 4\nline 5",
        ),
    ]);

    let area = Rect::new(0, 0, 80, 18);
    let mut pane_cache = cache();
    let mut grid = Grid::new(area);
    render(&mut grid, area, &lx, 0, &mut pane_cache);
    let text = rows_text(&grid, area).join("\n");
    assert!(text.contains("Permission switched x2"), "{text}");
    assert!(text.contains("mode: Full"), "{text}");
    assert!(
        text.contains("switched Read-only -> Workspace -> Full"),
        "{text}"
    );
    assert!(text.contains("more line"), "默认折叠保留预览: {text}");

    let hit =
        super::super::super::card_target_at(area, &lx, area.x + 6, area.y + 5, &mut pane_cache);
    assert!(
        matches!(
            hit,
            Some(crate::app::state::CardTarget::PermissionSwitch { .. })
        ),
        "点击面应命中权限切换提示: {hit:?}"
    );

    assert!(lx.permission_switch.toggle("psw-1"));
    let mut grid2 = Grid::new(area);
    render(&mut grid2, area, &lx, 0, &mut pane_cache);
    let expanded = rows_text(&grid2, area).join("\n");
    assert!(
        expanded.contains("line 5"),
        "展开后显示完整提示词: {expanded}"
    );
}

#[test]
fn merged_permission_switch_collapses_when_non_first_item_was_pre_expanded() {
    let mut lx = Lx::new();
    lx.chat.messages = vec![ChatMessageView {
        id: "u1".into(),
        sequence: 0,
        turn_index: 0,
        role: "user".into(),
        status: "complete".into(),
        kind: "chat".into(),
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: "hello".into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        reports: Vec::new(),
        attachments: Vec::new(),
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    }];
    lx.permission_switch.replace(vec![
        perm_switch(
            1,
            "read-only",
            "workspace-write",
            "Step 1\nline 2\nline 3\nline 4\nline 5",
        ),
        perm_switch(
            3,
            "workspace-write",
            "danger-full-access",
            "Step 2\nline 2\nline 3\nline 4\nline 5",
        ),
    ]);
    // 模拟区间中第 2 条记录曾单独展开，中间消息被删除后两项合并成 `Permission switched x2`。
    assert!(lx.permission_switch.toggle("psw-3"));

    let area = Rect::new(0, 0, 80, 20);
    let mut pane_cache = cache();
    let mut grid = Grid::new(area);
    render(&mut grid, area, &lx, 0, &mut pane_cache);
    let expanded_text = rows_text(&grid, area).join("\n");
    assert!(expanded_text.contains("Permission switched x2"));
    assert!(expanded_text.contains("line 5"), "{expanded_text}");

    let hit_y = (area.y..area.bottom())
        .find(|y| {
            (area.x..area.right())
                .map(|x| grid.cell(x, *y).ch)
                .collect::<String>()
                .contains("Step 2")
        })
        .expect("expanded prompt row");
    let hit = super::super::super::card_target_at(area, &lx, area.x + 6, hit_y, &mut pane_cache)
        .expect("hit permission switch card");
    let crate::app::state::CardTarget::PermissionSwitch { switch_id } = hit else {
        panic!("expected PermissionSwitch card target, got {hit:?}");
    };
    assert!(lx.permission_switch.toggle(&switch_id));

    let mut grid_collapsed = Grid::new(area);
    render(&mut grid_collapsed, area, &lx, 0, &mut pane_cache);
    let collapsed_text = rows_text(&grid_collapsed, area).join("\n");
    assert!(
        collapsed_text.contains("more line"),
        "点击合并卡片后必须正常折叠: {collapsed_text}"
    );
    assert!(!collapsed_text.contains("line 5"), "{collapsed_text}");

    // 若同区间有多条 ID 都在 expanded 中，app::update::toggle_permission_switch 也能一次全部收起。
    let mut state = crate::app::state::AppState::demo();
    let pane = state.active_tab().layout.focus();
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx = lx;
        target.lx.permission_switch.toggle("psw-1");
        target.lx.permission_switch.toggle("psw-3");
    }
    assert!(crate::app::update::toggle_permission_switch(
        &mut state, pane, "psw-1"
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(
        target.lx.permission_switch.expanded.is_empty(),
        "合并区间点击折叠应清空同组所有已展开 ID"
    );
}
