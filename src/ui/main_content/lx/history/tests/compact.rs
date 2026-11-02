//! compact 检查点条目：折叠/展开、与其他条目交错、窄宽换行。

use super::*;

fn checkpoint(sequence: i64, active: bool) -> crate::app::state::CompactNoticeView {
    crate::app::state::CompactNoticeView {
        id: format!("compact-{sequence}"),
        sequence,
        model_label: "gpt-5".into(),
        created_at_ms: 1_700_000_640_000,
        start_sequence: 0,
        end_sequence: sequence.saturating_sub(1),
        summary: "Condensed context for the earlier work.".into(),
        active,
        auto: false,
    }
}

/// 只有检查点时仍显示历史（不退回欢迎页）。
#[test]
fn compact_only_history_stays_on_history_view() {
    let mut lx = Lx::new();
    lx.compact.replace(vec![checkpoint(1, true)]);
    assert!(has_history(&lx));
    let area = Rect::new(0, 0, 60, 10);
    let view = view(area, &lx, &mut cache());
    assert!(view.total_rows > 0);
}

/// 折叠态：元信息常显，短摘要按预览行展示（未超出预算时不登记点击面）。
#[test]
fn compact_entry_folds_to_header_and_meta_rows() {
    let mut lx = Lx::new();
    lx.compact.replace(vec![checkpoint(1, true)]);
    let area = Rect::new(0, 0, 60, 10);
    let layout = layout(area, &lx, &mut cache());
    let mut grid = Grid::new(area);
    let drawn = render(&mut grid, area, &lx, 0, &mut cache());
    assert_eq!(drawn, layout.total_rows, "布局与渲染行数必须一致");
    // 标题 + 模型 + 覆盖范围 + 摘要（1 行未超预算）+ 末尾呼吸行。
    assert_eq!(layout.total_rows, 5, "折叠态只占标题+元信息+摘要+呼吸行");
    assert_eq!(
        layout.cards.first().map(|card| card.clickable),
        Some(false),
        "正文未超出折叠预算时不登记点击面"
    );
    let text: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| grid.cell(x, y).ch)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Context compacted"), "{text}");
    assert!(text.contains('◆'), "系统事件包含菱形标记：{text}");
    assert!(
        text.contains("  ├─ model: gpt-5"),
        "模型行挂在 `├─` 下：{text}"
    );
    assert!(
        text.contains("  ├─ covered sequence 0"),
        "覆盖范围默认可见：{text}"
    );
    assert!(
        text.contains("  └─ Condensed context"),
        "摘要节点由 `└─` 收口：{text}"
    );
}

/// 长摘要折叠为预览 + 剩余行数；展开后铺开全部内容。
#[test]
fn compact_content_folds_to_preview_and_expands() {
    let mut lx = Lx::new();
    let mut notice = checkpoint(4, true);
    notice.summary = "Condensed line one.\nLine two.\nLine three.\nLine four.".into();
    lx.compact.replace(vec![notice]);
    let area = Rect::new(0, 0, 60, 20);
    let folded = layout(area, &lx, &mut cache()).total_rows;
    let mut grid = Grid::new(area);
    let drawn = render(&mut grid, area, &lx, 0, &mut cache());
    assert_eq!(drawn, folded, "折叠态布局与渲染行数必须一致");
    assert_eq!(
        layout(area, &lx, &mut cache())
            .cards
            .first()
            .map(|card| card.clickable),
        Some(true),
        "正文超出折叠预算时必须登记点击面"
    );
    let folded_text: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| grid.cell(x, y).ch)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(folded_text.contains("Condensed line one"), "{folded_text}");
    assert!(
        !folded_text.contains("Line four"),
        "折叠态不绘制预算之外的正文：{folded_text}"
    );
    assert!(
        folded_text.contains("… 2 more lines"),
        "折叠提示给出隐藏行数：{folded_text}"
    );

    lx.compact.toggle("compact-4");
    let expanded = layout(area, &lx, &mut cache()).total_rows;
    let mut grid2 = Grid::new(area);
    let drawn2 = render(&mut grid2, area, &lx, 0, &mut cache());
    assert_eq!(drawn2, expanded, "展开后仍必须一致");
    assert!(expanded > folded, "展开必须增加行数");
    let expanded_text: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| grid2.cell(x, y).ch)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(expanded_text.contains("Line four"), "{expanded_text}");
    assert!(
        !expanded_text.contains("more lines"),
        "展开后不再出现折叠提示：{expanded_text}"
    );
}

/// 检查点与消息按 sequence 排序合并（不插在末尾）。
#[test]
fn compact_entry_is_interleaved_by_sequence() {
    let mut lx = lx_with(vec![message(
        "assistant",
        "complete",
        vec![text_part("after")],
    )]);
    lx.chat.messages[0].sequence = 5;
    lx.compact.replace(vec![checkpoint(2, true)]);
    let area = Rect::new(0, 0, 60, 16);
    let mut grid = Grid::new(area);
    render(&mut grid, area, &lx, 0, &mut cache());
    let text: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| grid.cell(x, y).ch)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let compact_at = text.find("Context compacted").expect("compact title");
    let message_at = text.find("after").expect("message body");
    assert!(
        compact_at < message_at,
        "sequence 更小的检查点必须排在消息之前：{text}"
    );
}

/// 连续多个检查点各自独立（不合并成 xN）。
#[test]
fn consecutive_compacts_are_not_merged() {
    let mut lx = Lx::new();
    lx.compact
        .replace(vec![checkpoint(1, true), checkpoint(2, true)]);
    let area = Rect::new(0, 0, 60, 12);
    let mut grid = Grid::new(area);
    render(&mut grid, area, &lx, 0, &mut cache());
    let text: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| grid.cell(x, y).ch)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        text.matches("Context compacted").count(),
        2,
        "每条检查点都需要独立展开：{text}"
    );
}

/// 已撤销的检查点保留在时间线上但明确标注。
#[test]
fn reverted_checkpoint_is_marked() {
    let mut lx = Lx::new();
    lx.compact.replace(vec![checkpoint(3, false)]);
    let area = Rect::new(0, 0, 60, 10);
    let mut grid = Grid::new(area);
    render(&mut grid, area, &lx, 0, &mut cache());
    let text: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| grid.cell(x, y).ch)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("reverted"), "{text}");
}

/// 窄屏：compact 条目按内容宽度换行，布局与渲染行数仍必须一致。
#[test]
fn compact_entry_wraps_on_narrow_width() {
    let mut lx = Lx::new();
    let mut notice = checkpoint(4, true);
    notice.summary =
        "a long summary line that will certainly need to wrap on narrow screens".into();
    lx.compact.replace(vec![notice]);
    lx.compact.toggle("compact-4");
    for width in [16u16, 24, 40, 80] {
        let area = Rect::new(0, 0, width, 30);
        let layout = layout(area, &lx, &mut cache());
        let mut grid = Grid::new(area);
        let drawn = render(&mut grid, area, &lx, 0, &mut cache());
        assert_eq!(
            drawn, layout.total_rows,
            "宽度 {width} 下行数必须一致（换行不得导致滚动漂移）"
        );
        assert!(layout.total_rows > 3, "宽度 {width} 下展开条目应有内容");
    }
}

/// 折叠/展开切换后布局行数随之变化，且锚点由 `pin_lx_viewport` 之外的层处理。
#[test]
fn compact_fold_toggle_changes_row_count_consistently() {
    let mut lx = Lx::new();
    let mut notice = checkpoint(4, true);
    notice.summary = "One.\nTwo.\nThree.\nFour.".into();
    lx.compact.replace(vec![notice]);
    let area = Rect::new(0, 0, 50, 20);
    let folded = layout(area, &lx, &mut cache()).total_rows;
    lx.compact.toggle("compact-4");
    let expanded = layout(area, &lx, &mut cache()).total_rows;
    assert!(expanded > folded, "展开必须增加行数");
    let mut grid = Grid::new(area);
    assert_eq!(render(&mut grid, area, &lx, 0, &mut cache()), expanded);
}

#[test]
fn expanded_compact_summary_uses_muted_markdown_styles() {
    let mut lx = Lx::new();
    let mut notice = checkpoint(4, true);
    notice.summary = "Summary with `inline_code` token".into();
    lx.compact.replace(vec![notice]);
    lx.compact.toggle("compact-4");
    let area = Rect::new(0, 0, 60, 12);
    let mut grid = Grid::new(area);
    let drawn = render(&mut grid, area, &lx, 0, &mut cache());
    assert_eq!(drawn, layout(area, &lx, &mut cache()).total_rows);
    let summary_y = (area.y..area.bottom())
        .find(|&y| {
            let line: String = (area.x..area.right()).map(|x| grid.cell(x, y).ch).collect();
            line.contains("Summary with inline_code")
        })
        .expect("summary row");
    assert_eq!(grid.cell(5, summary_y).style, style::muted());
    assert_eq!(
        grid.cell(18, summary_y).style,
        style::md_read_inline_code().patch(style::muted())
    );
}

/// 正在执行的压缩请求在时间线末尾显示动态呼吸条目及耗时。
#[test]
fn running_compact_displays_breathing_glyph_and_elapsed() {
    let mut lx = Lx::new();
    lx.compact.start("compact-op-1".into(), 1_000);
    lx.compact.elapsed_ms = Some(3_200);
    assert!(has_history(&lx), "压缩进行中应当有可绘制历史");

    let area = Rect::new(0, 0, 60, 10);
    let layout = layout(area, &lx, &mut cache());
    // 运行条目 1 行 + 末尾呼吸位 1 行 = 2 行
    assert_eq!(layout.total_rows, 2, "运行中只占单行条目+呼吸行");

    // 相位 0（实心菱形）
    lx.chat.loading_frame = 0;
    let mut grid = Grid::new(area);
    let drawn = render(&mut grid, area, &lx, 0, &mut cache());
    assert_eq!(drawn, layout.total_rows);
    let text: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| grid.cell(x, y).ch)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Compacting context..."), "{text}");
    assert!(text.contains('◆'), "相位 0 必须为实心菱形：{text}");
    assert!(text.contains("3.2s"), "耗时格式化为秒：{text}");

    // 相位 4（呼吸到空心菱形）
    lx.chat.loading_frame = crate::ui::text::agent::STEP_BREATH_TICKS;
    let mut grid_hollow = Grid::new(area);
    render(&mut grid_hollow, area, &lx, 0, &mut cache());
    let text_hollow: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| grid_hollow.cell(x, y).ch)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text_hollow.contains('◇'),
        "呼吸相位必须为空心菱形：{text_hollow}"
    );
}
