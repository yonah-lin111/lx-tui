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
