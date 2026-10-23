//! 扫描动画（A18）：按可见文字推进、窗口 3 字符、统计不参与扫描。

use super::*;

// ===== 动画（A18）=====

/// A18：可见文字继续扫描；统计始终保持 dim（不随帧闪动）。
#[test]
fn scanning_uses_visible_text_and_stats_stay_static() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    lx.chat.plan = Some(plan(&[(
        "一个非常长的中文任务描述需要被截断",
        PlanStepView::InProgress,
    )]));
    with_stats(&mut lx);
    let stats_w = u16::try_from(STATS.width()).unwrap_or(0);
    let width = stats_w + 2 + 1 + 8; // 文字预算 8 列：可见文字更短，扫描周期按可见文字算。
    let mut highlighted = 0usize;
    for frame in 0..24 {
        lx.chat.loading_frame = frame;
        let (grid, bar) = draw_bar(&lx, width);
        let row = row_text(&grid, bar);
        let styles = row_styles(&grid, bar);
        let stats_col = col_of(&row, STATS);
        let stats_end = stats_col + u16::try_from(STATS.width()).unwrap_or(0);
        for (col, style) in styles.iter().enumerate() {
            let col = u16::try_from(col).unwrap_or(u16::MAX);
            let bold = style.add_modifier.contains(ratatui::style::Modifier::BOLD);
            if (stats_col..stats_end).contains(&col) {
                assert!(!bold, "frame {frame}: 统计不参与扫描：{row:?}");
                assert!(
                    style.add_modifier.contains(ratatui::style::Modifier::DIM),
                    "frame {frame}: 统计恒为 dim"
                );
            } else if bold {
                highlighted += 1;
            }
        }
    }
    assert!(highlighted > 0, "可见文字仍有扫描高亮");
}

/// A18 续：扫描窗口不超过 3 个字符，且每个被扫字符取当前帧色。
#[test]
fn scan_window_stays_three_cells() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    lx.chat.plan = Some(plan(&[("abcdefghijkl", PlanStepView::InProgress)]));
    let cycle = crate::ui::text::agent::native_label_cycle();
    for frame in 0..10 {
        lx.chat.loading_frame = frame;
        let (grid, bar) = draw_bar(&lx, 40);
        let styles = row_styles(&grid, bar);
        let hits: Vec<usize> = styles
            .iter()
            .enumerate()
            .filter(|(_, style)| style.add_modifier.contains(ratatui::style::Modifier::BOLD))
            .map(|(index, _)| index)
            .collect();
        if hits.is_empty() {
            continue;
        }
        assert!(hits.len() <= SCAN_WINDOW, "frame {frame}: {hits:?}");
        assert!(
            hits.windows(2).all(|pair| pair[1] == pair[0] + 1),
            "frame {frame}: 窗口不连续 {hits:?}"
        );
        let expected = cycle[frame % cycle.len()];
        for index in hits {
            assert_eq!(
                styles[index].fg,
                Some(ratatui::style::Color::Indexed(expected)),
                "frame {frame}"
            );
        }
    }
}
