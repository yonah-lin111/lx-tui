//! `lx` 消息列表 Markdown 表格单元格长按拖选、局部反显高亮、防手抖吸附、双击/三击与去边框复制测试。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::app::selection::{Selection, SelectionUnit};
use crate::app::state::{
    AssistantStepView, ChatMessageView, ChatPartView, Lx, LxChat, SubagentReportView, SubagentRun,
    TaskPage, TaskTab,
};
use crate::chat::agent::AgentRole;
use crate::chat::events::PartKind;
use crate::layout::PaneId;

use super::super::{copy_text, render};
use super::history::PaneCache;
use super::{content_geom, unit_cell_range};

/// 构造单条 assistant Markdown 消息的 `Lx` 状态。
fn assistant_markdown_lx(markdown: &str) -> Lx {
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages.push(ChatMessageView {
        sequence: 0,
        id: "a0".into(),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: markdown.into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    });
    lx.chat = chat;
    lx
}

#[test]
fn single_cell_multiline_soft_wrap_selection_highlights_and_copies_only_target_cell() {
    let md = "| 序号 | 名称 | 特点 |\n| --- | --- | --- |\n| 1 | DeepSeek | 免费、支持长上下文与代码推理 |\n| 2 | Rust | 内存安全、零成本抽象 |";
    let area = Rect::new(0, 0, 34, 18);
    let lx = assistant_markdown_lx(md);
    let mut cache = PaneCache::default();

    // 先渲染一次以使快照就绪，并定位第 1 条数据行（DeepSeek 行）在网格中的多行单元格坐标。
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &lx, true, None, &mut cache);
    let geom = content_geom(area, &lx, &mut cache).expect("content geom");

    let chunk_rect = Rect::new(0, 0, geom.rect.width, geom.rect.height);
    let mut grid = super::super::grid::Grid::new(chunk_rect);
    super::render_chunk(&mut grid, chunk_rect, &lx, 0, &mut cache);

    // 找到 row_idx == 1（第一条数据行）的起止物理行与第 2 列（特点列）的内容区间。
    let first_table_row = grid.table_row_at(0).expect("table row at 0");
    let meta = &first_table_row.line_ref.table;
    let base_col = first_table_row.base_col;
    let logical_row1 = meta.logical_rows[1];
    assert!(
        logical_row1.last_line > logical_row1.first_line,
        "窄宽度下『特点』列应自动折成多行"
    );
    let col2 = meta.columns[2];
    let col1 = meta.columns[1];

    let start_cell = (
        i32::try_from(logical_row1.first_line).unwrap_or(0),
        base_col + col2.content_start,
    );
    let end_cell = (
        i32::try_from(logical_row1.last_line).unwrap_or(0),
        base_col + col2.content_end - 1,
    );

    // 1. 验证复制结果：软折行无缝拼接，绝不含相邻列（"1"、"DeepSeek"）或 `│` 边框，也不插入多余 `\n`。
    let copied = copy_text(area, &lx, (start_cell, end_cell), &mut cache);
    assert_eq!(copied, "免费、支持长上下文与代码推理");

    // 2. 验证反显高亮：仅第 2 列的真实文本列被 REVERSED，左侧相邻列与 `│` 竖线均未反显。
    let sel = Selection::text_range(PaneId::from_raw_for_test(1), start_cell, end_cell);
    let mut sel_buf = Buffer::empty(area);
    render(area, &mut sel_buf, &lx, true, Some(&sel), &mut cache);

    for line_off in logical_row1.first_line..=logical_row1.last_line {
        let y = geom.rect.y + u16::try_from(line_off).unwrap_or(0);
        // 左侧 `│` 边框与第 1 列（DeepSeek）绝不能被反显。
        let border_x = geom.rect.x + base_col + col2.box_start - 1;
        assert!(
            !sel_buf[(border_x, y)].modifier.contains(Modifier::REVERSED),
            "竖线边框列不得被反显 (line_off={line_off})"
        );
        let col1_x = geom.rect.x + base_col + col1.content_start;
        assert!(
            !sel_buf[(col1_x, y)].modifier.contains(Modifier::REVERSED),
            "相邻列不得被反显 (line_off={line_off})"
        );
        // 目标单元格非填充物理行的首字符应当被反显，空白填充行不得被反显。
        let is_filler = match &meta.lines[line_off] {
            crate::ui::markdown_render::TableLineKind::Cells { cells, .. } => cells[2].is_filler,
            _ => true,
        };
        let target_x = geom.rect.x + base_col + col2.content_start;
        if is_filler {
            assert!(
                !sel_buf[(target_x, y)].modifier.contains(Modifier::REVERSED),
                "目标单元格空白填充行不得被反显 (line_off={line_off})"
            );
        } else {
            assert!(
                sel_buf[(target_x, y)].modifier.contains(Modifier::REVERSED),
                "目标单元格文本应被反显 (line_off={line_off})"
            );
        }
    }
}

#[test]
fn single_cell_english_soft_wrap_preserves_inter_word_spaces_and_br_keeps_newline() {
    let md = "| 命令 | 详情 |\n| --- | --- |\n| 测试 | cargo test --all-targets |\n| 多行 | 第一行<br>第二行<br/>第三行 |";
    let area = Rect::new(0, 0, 28, 20);
    let lx = assistant_markdown_lx(md);
    let mut cache = PaneCache::default();

    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &lx, true, None, &mut cache);
    let geom = content_geom(area, &lx, &mut cache).expect("content geom");
    let chunk_rect = Rect::new(0, 0, geom.rect.width, geom.rect.height);
    let mut grid = super::super::grid::Grid::new(chunk_rect);
    super::render_chunk(&mut grid, chunk_rect, &lx, 0, &mut cache);

    let first_table_row = grid.table_row_at(0).expect("table row");
    let meta = &first_table_row.line_ref.table;
    let base_col = first_table_row.base_col;
    let col1 = meta.columns[1];

    // 行 1（英文软折行）：应无缝还原 "cargo test --all-targets"，保留词间空格且无 `\n`。
    let row1 = meta.logical_rows[1];
    assert!(
        row1.last_line > row1.first_line,
        "英文命令在窄列宽下应软折行"
    );
    let copied_soft = copy_text(
        area,
        &lx,
        (
            (
                i32::try_from(row1.first_line).unwrap_or(0),
                base_col + col1.content_start,
            ),
            (
                i32::try_from(row1.last_line).unwrap_or(0),
                base_col + col1.content_end - 1,
            ),
        ),
        &mut cache,
    );
    assert_eq!(copied_soft, "cargo test --all-targets");

    // 行 2（显式 `<br>` 换行）：应保留 `\n`。
    let row2 = meta.logical_rows[2];
    let copied_hard = copy_text(
        area,
        &lx,
        (
            (
                i32::try_from(row2.first_line).unwrap_or(0),
                base_col + col1.content_start,
            ),
            (
                i32::try_from(row2.last_line).unwrap_or(0),
                base_col + col1.content_end - 1,
            ),
        ),
        &mut cache,
    );
    assert_eq!(copied_hard, "第一行\n第二行\n第三行");
}

#[test]
fn column_locked_and_grid_modes_format_cells_with_tabs_and_newlines_without_borders() {
    let md = "| 姓名 | 角色 | 状态 |\n| --- | --- | --- |\n| 张三 | 后端开发 | 在线 |\n| 李四 | 前端设计 | 离线 |\n| 王五 | 架构师 | 在线 |";
    let area = Rect::new(0, 0, 50, 20);
    let lx = assistant_markdown_lx(md);
    let mut cache = PaneCache::default();

    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &lx, true, None, &mut cache);
    let geom = content_geom(area, &lx, &mut cache).expect("content geom");
    let chunk_rect = Rect::new(0, 0, geom.rect.width, geom.rect.height);
    let mut grid = super::super::grid::Grid::new(chunk_rect);
    super::render_chunk(&mut grid, chunk_rect, &lx, 0, &mut cache);

    let first_table_row = grid.table_row_at(0).expect("table row");
    let meta = &first_table_row.line_ref.table;
    let base_col = first_table_row.base_col;

    // 1. ColumnLocked：在第 1 列（角色列）从第 1 条数据行拖选到第 3 条数据行。
    let col1 = meta.columns[1];
    let r1_line = i32::try_from(meta.logical_rows[1].first_line).unwrap_or(0);
    let r3_line = i32::try_from(meta.logical_rows[3].last_line).unwrap_or(0);
    let col_locked = copy_text(
        area,
        &lx,
        (
            (r1_line, base_col + col1.content_start + 1),
            (r3_line, base_col + col1.content_start + 1),
        ),
        &mut cache,
    );
    assert_eq!(col_locked, "后端开发\n前端设计\n架构师");

    // 2. Grid：从表头第 0 列拖选到末行第 2 列（整表选中）。
    let col0 = meta.columns[0];
    let col2 = meta.columns[2];
    let r0_line = i32::try_from(meta.logical_rows[0].first_line).unwrap_or(0);
    let grid_copied = copy_text(
        area,
        &lx,
        (
            (r0_line, base_col + col0.content_start),
            (r3_line, base_col + col2.content_end - 1),
        ),
        &mut cache,
    );
    assert_eq!(
        grid_copied,
        "姓名\t角色\t状态\n张三\t后端开发\t在线\n李四\t前端设计\t离线\n王五\t架构师\t在线"
    );
}

#[test]
fn adjacent_border_snapping_and_cross_block_selection_work_cleanly() {
    let md = "前置说明段落\n\n| 模块 | 描述 |\n| --- | --- |\n| 核心 | 负责调度与状态同步 |\n\n后置总结段落";
    let area = Rect::new(0, 0, 32, 20);
    let lx = assistant_markdown_lx(md);
    let mut cache = PaneCache::default();

    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &lx, true, None, &mut cache);
    let geom = content_geom(area, &lx, &mut cache).expect("content geom");
    let chunk_rect = Rect::new(0, 0, geom.rect.width, geom.rect.height);
    let mut grid = super::super::grid::Grid::new(chunk_rect);
    super::render_chunk(&mut grid, chunk_rect, &lx, 0, &mut cache);

    // 找到表格顶边框所在行
    let table_top_y = (0..geom.rect.height)
        .find(|&y| grid.table_row_at(y).is_some())
        .expect("table top y");
    let table_row = grid.table_row_at(table_top_y).expect("table row");
    let meta = &table_row.line_ref.table;
    let base_col = table_row.base_col;
    let table_start = i32::from(table_top_y);

    // 1. 防手抖测试：起点在 (row_idx=1, col_idx=1) 内容区，终点压在该单元格右侧 `│` 边框及下方 `└─┴─┘` 底边框上。
    let row1 = meta.logical_rows[1];
    let col1 = meta.columns[1];
    let anchor = (
        table_start + i32::try_from(row1.first_line).unwrap_or(0),
        base_col + col1.content_start,
    );
    let cursor_on_bottom_right_border = (
        table_start + i32::try_from(row1.last_line + 1).unwrap_or(0),
        base_col + col1.box_end,
    );
    let snapped_copy = copy_text(
        area,
        &lx,
        (anchor, cursor_on_bottom_right_border),
        &mut cache,
    );
    assert_eq!(
        snapped_copy, "负责调度与状态同步",
        "终点压在单元格自身紧邻边框上时应吸附回该单元格"
    );

    // 2. 跨块拖选：从前置段落一直拖选到后置段落。
    let full_copy = copy_text(
        area,
        &lx,
        (
            (0, 0),
            (i32::try_from(geom.total_rows - 1).unwrap_or(0), 30),
        ),
        &mut cache,
    );
    assert_eq!(
        full_copy,
        "● 前置说明段落\n模块\t描述\n核心\t负责调度与状态同步\n  后置总结段落"
    );
}

#[test]
fn double_click_word_and_triple_click_cell_respect_table_cell_bounds() {
    let md = "| 语言 | 特性 |\n| --- | --- |\n| Go | Rust 内存安全、并发无数据竞争且性能优异 |";
    let area = Rect::new(0, 0, 36, 18);
    let lx = assistant_markdown_lx(md);
    let mut cache = PaneCache::default();

    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &lx, true, None, &mut cache);
    let geom = content_geom(area, &lx, &mut cache).expect("content geom");
    let chunk_rect = Rect::new(0, 0, geom.rect.width, geom.rect.height);
    let mut grid = super::super::grid::Grid::new(chunk_rect);
    super::render_chunk(&mut grid, chunk_rect, &lx, 0, &mut cache);

    let first_table_row = grid.table_row_at(0).expect("table row");
    let meta = &first_table_row.line_ref.table;
    let base_col = first_table_row.base_col;
    let row1 = meta.logical_rows[1];

    // 1. 双击第 1 列首词 ("Rust")：选词区间不得包含左侧 `│` 边框、留白或后续中文。
    let col1_word_click = (
        i32::try_from(row1.first_line).unwrap_or(0),
        base_col + meta.columns[1].content_start,
    );
    let word_range = unit_cell_range(area, &lx, col1_word_click, SelectionUnit::Word, &mut cache);
    let word_copied = copy_text(area, &lx, word_range, &mut cache);
    assert_eq!(word_copied, "Rust");

    // 2. 三击第 1 列（折成多行的长单元格）：直接选中整格全部多行内容。
    let col1_click = (
        i32::try_from(row1.first_line).unwrap_or(0),
        base_col + meta.columns[1].content_start + 1,
    );
    let cell_range = unit_cell_range(area, &lx, col1_click, SelectionUnit::Line, &mut cache);
    let cell_copied = copy_text(area, &lx, cell_range, &mut cache);
    assert_eq!(cell_copied, "Rust 内存安全、并发无数据竞争且性能优异");
}

#[test]
fn table_selection_works_in_thinking_reports_and_subagent_detail_views() {
    let table_md =
        "| 阶段 | 结论 |\n| --- | --- |\n| 验证 | 思考块与子智能体表格单格多行复制正常 |";
    let area = Rect::new(0, 0, 36, 30);

    // 1. 汇报块消息 + 思考块消息中的表格
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages.push(ChatMessageView {
        sequence: 0,
        id: "r0".into(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: Vec::new(),
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: vec![SubagentReportView {
            agent_id: "sub-1".into(),
            nickname: None,
            title: "检查表格".into(),
            role: AgentRole::Explorer,
            status: "completed".into(),
            files_changed: 0,
            changed_paths: Vec::new(),
            duration_ms: Some(500),
            body: table_md.into(),
            delivered_at_ms: 1,
        }],
        attachments: Vec::new(),
    });
    chat.messages.push(ChatMessageView {
        sequence: 1,
        id: "a0".into(),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Reasoning,
                content: table_md.into(),
                provider_metadata_json: None,
                started_at_ms: Some(1000),
                ended_at_ms: Some(2000),
                completed_at_ms: Some(2000),
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    });
    // 展开汇报块与思考块
    chat.toggle_detail("r0", "report:sub-1");
    chat.toggle_reasoning(1, 0);
    lx.chat = chat;

    let mut cache = PaneCache::default();
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &lx, true, None, &mut cache);
    let geom = content_geom(area, &lx, &mut cache).expect("geom");
    let chunk_rect = Rect::new(0, 0, geom.rect.width, geom.rect.height);
    let mut grid = super::super::grid::Grid::new(chunk_rect);
    super::render_chunk(&mut grid, chunk_rect, &lx, 0, &mut cache);

    // 收集所有表格顶边框行（第 1 个属于汇报块，第 2 个属于思考块）
    let table_tops: Vec<u16> = (0..geom.rect.height)
        .filter(|&y| {
            grid.table_row_at(y)
                .is_some_and(|tr| tr.line_ref.line_offset == 0)
        })
        .collect();
    assert_eq!(table_tops.len(), 2, "思考块与汇报块均应登记表格几何");

    for top_y in table_tops {
        let tr = grid.table_row_at(top_y).expect("table row");
        let row1 = tr.line_ref.table.logical_rows[1];
        let col1 = tr.line_ref.table.columns[1];
        let click = (
            i32::from(top_y) + i32::try_from(row1.first_line).unwrap_or(0),
            tr.base_col + col1.content_start,
        );
        let range = unit_cell_range(area, &lx, click, SelectionUnit::Line, &mut cache);
        let copied = copy_text(area, &lx, range, &mut cache);
        assert_eq!(copied, "思考块与子智能体表格单格多行复制正常");
    }

    // 2. TaskTab::Agents 子智能体详情页中的表格
    let mut agent_lx = Lx::new();
    let mut run = SubagentRun::from_record(&crate::db::chat::SubagentRecord {
        agent_id: "agent-1".into(),
        nickname: None,
        agent_path: "/root/agent".into(),
        child_session_id: "sess-1".into(),
        parent_session_id: "parent".into(),
        spawn_message_id: None,
        role: AgentRole::Explorer,
        model_provider: String::new(),
        model_id: String::new(),
        model_variant: None,
        status: "completed".into(),
        title: "table-task".into(),
        created_at_ms: 0,
        completed_at_ms: Some(100),
        report_json: None,
        report_delivered: false,
    });
    run.loaded = true;
    run.chat.messages = assistant_markdown_lx(table_md).chat.messages;
    agent_lx.agents.runs.push(run);
    agent_lx.task = Some(TaskPage {
        tab: TaskTab::Agents,
        ..Default::default()
    });

    let mut agent_cache = PaneCache::default();
    let mut agent_buf = Buffer::empty(area);
    render(
        area,
        &mut agent_buf,
        &agent_lx,
        true,
        None,
        &mut agent_cache,
    );
    let agent_geom = content_geom(area, &agent_lx, &mut agent_cache).expect("agent geom");
    let chunk_rect = Rect::new(0, 0, agent_geom.rect.width, agent_geom.rect.height);
    let mut agent_grid = super::super::grid::Grid::new(chunk_rect);
    super::render_chunk(&mut agent_grid, chunk_rect, &agent_lx, 0, &mut agent_cache);

    let tr = agent_grid.table_row_at(0).expect("agent table row");
    let row1 = tr.line_ref.table.logical_rows[1];
    let col1 = tr.line_ref.table.columns[1];
    let click = (
        i32::try_from(row1.first_line).unwrap_or(0),
        tr.base_col + col1.content_start,
    );
    let range = unit_cell_range(
        area,
        &agent_lx,
        click,
        SelectionUnit::Line,
        &mut agent_cache,
    );
    let copied = copy_text(area, &agent_lx, range, &mut agent_cache);
    assert_eq!(copied, "思考块与子智能体表格单格多行复制正常");
}
