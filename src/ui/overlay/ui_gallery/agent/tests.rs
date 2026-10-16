//! 单元测试：token 格式化、样例行展开、尺寸与单套（lx）样式渲染。

use super::*;

use ratatui::buffer::Buffer;

use crate::app::ui_gallery::AgentStepId;

/// 渲染一个步骤的整块预览并返回文本（行以 `\n` 连接）。
fn render_step(step: AgentStepId, width: u16, height: u16) -> String {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    let area = buf.area;
    render_agent_appearance(&mut buf, area, step);
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 取一行里的全部文本。
fn line_text(line: &SampleLine) -> String {
    line.spans.iter().map(|(value, _)| value.as_str()).collect()
}

#[test]
fn format_tokens_matches_lx_agent_rules() {
    assert_eq!(format_tokens(0), "0");
    assert_eq!(format_tokens(940), "940");
    assert_eq!(format_tokens(999), "999");
    // <10000：一位小数。
    assert_eq!(format_tokens(1_000), "1.0k");
    assert_eq!(format_tokens(4_280), "4.3k");
    assert_eq!(format_tokens(9_999), "10.0k");
    // <1M：四舍五入取整。
    assert_eq!(format_tokens(10_000), "10k");
    assert_eq!(format_tokens(12_400), "12k");
    assert_eq!(format_tokens(999_499), "999k");
    // ≥1M：一位小数。
    assert_eq!(format_tokens(1_000_000), "1.0M");
    assert_eq!(format_tokens(1_240_000), "1.2M");
}

#[test]
fn token_footer_omits_cache_when_zero() {
    let line = format_tokens_line((940, 120, 0));
    assert!(line.contains(text::agent::TOKEN_IN));
    assert!(line.contains(text::agent::TOKEN_OUT));
    assert!(!line.contains(text::agent::TOKEN_CACHE), "{line}");

    let with_cache = format_tokens_line((1_240, 80, 620));
    assert!(with_cache.contains(text::agent::TOKEN_CACHE));
    assert!(with_cache.contains("1.2k"));
}

#[test]
fn token_footer_uses_separator_between_segments() {
    let line = format_tokens_line((12_400, 312, 8_100));
    let sep = text::agent::SEPARATOR.trim();
    let parts: Vec<&str> = line.split(sep).map(str::trim).collect();
    assert_eq!(parts.len(), 3, "IN/OUT/CACHE 三段：{line}");
    assert!(parts[0].starts_with(text::agent::TOKEN_IN));
    assert!(parts[1].starts_with(text::agent::TOKEN_OUT));
    assert!(parts[2].starts_with(text::agent::TOKEN_CACHE));
}

#[test]
fn every_step_has_header_and_a_token_footer() {
    for step in AgentStepId::ALL {
        let lines = step_lines(step);
        assert!(lines.len() >= 2, "步骤 {step:?} 至少含头部与 token 页脚");
        if step == AgentStepId::Tokens {
            continue; // 聚合 item 用汇总条，下面单独断言。
        }
        let footer = line_text(lines.last().expect("non-empty"));
        assert!(
            footer.contains(text::agent::TOKEN_IN),
            "步骤 {step:?} 末行应为 token 页脚：{footer}"
        );
    }
}

#[test]
fn tokens_item_shows_turn_summary_and_context_pills() {
    let lines = step_lines(AgentStepId::Tokens);
    let joined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
    assert!(joined.contains(text::agent::TOKENS_BAR_LEAD));
    assert!(joined.contains(text::agent::TOKEN_IN));
    assert!(joined.contains("42%"));
    assert!(joined.contains("78%"));
    assert!(joined.contains("92%"));
}

#[test]
fn subagent_shows_single_in_progress_row_and_one_total_token_line() {
    let lines = step_lines(AgentStepId::Subagent);
    let joined: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
    // 只显示“进行中”的那一条子任务。
    assert!(
        joined.contains(text::agent::SUBAGENT_IN_PROGRESS),
        "应显示进行中的子任务"
    );
    // token 只出现一行（底部总计）。
    let with_in = lines
        .iter()
        .filter(|line| line_text(line).contains(text::agent::TOKEN_IN))
        .count();
    assert_eq!(with_in, 1, "只应有一行 token 页脚");
}

#[test]
fn error_title_is_red_and_bold() {
    let header = header_line(AgentStepId::Error);
    let red = style::agent_fail();
    // 标题各段（图标、动词、参数）应全为失败红。
    assert!(!header.spans.is_empty());
    assert!(
        header.spans.iter().all(|(_, style)| *style == red),
        "error 标题应整行红+加粗：{:?}",
        header.spans
    );
    // 非 error 步骤的标题不用失败红。
    let ok = header_line(AgentStepId::Exec);
    assert!(
        ok.spans
            .iter()
            .all(|(_, style)| *style != style::agent_fail())
    );
}

#[test]
fn header_colors_icon_verb_and_param_distinctly() {
    let header = header_line(AgentStepId::Exec);
    // 图标品牌粉、动词天蓝、参数冰蓝+下划线：三者互不相同。
    assert_eq!(header.spans[0].1, style::agent_icon());
    assert_eq!(header.spans[1].1, style::agent_verb());
    assert_eq!(header.spans[2].1, style::agent_param());
    assert!(
        header.spans[2]
            .1
            .add_modifier
            .contains(ratatui::style::Modifier::UNDERLINED),
        "可变参数应带下划线"
    );
    // 三者前景色两两不同。
    let fg = |s: &Style| s.fg;
    let colors = [
        fg(&header.spans[0].1),
        fg(&header.spans[1].1),
        fg(&header.spans[2].1),
    ];
    assert_ne!(colors[0], colors[1]);
    assert_ne!(colors[1], colors[2]);
    assert_ne!(colors[0], colors[2]);
}

#[test]
fn todo_uses_claude_checkbox_trio() {
    let joined: String = step_lines(AgentStepId::Todo)
        .iter()
        .map(line_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains(text::agent::TODO_DONE));
    assert!(joined.contains(text::agent::TODO_IN_PROGRESS));
    assert!(joined.contains(text::agent::TODO_PENDING));
}

#[test]
fn appearance_size_fits_content_and_clamps_to_available() {
    let available = Rect::new(0, 0, 80, 60);
    for step in AgentStepId::ALL {
        let (width, height) = agent_appearance_size(step, available);
        assert!((6..=available.width).contains(&width));
        let expected = step_lines(step).len() as u16;
        assert_eq!(height, expected.min(available.height));
    }
    let tiny = Rect::new(0, 0, 80, 3);
    let (_, height) = agent_appearance_size(AgentStepId::Tokens, tiny);
    assert_eq!(height, 3);
}

#[test]
fn render_draws_icon_verb_param_and_token_footer() {
    let all = render_step(AgentStepId::Exec, 60, 50);
    // lx 图标、直角连接符与 token 页脚都出现。
    assert!(all.contains(text::agent::ICON_EXEC));
    assert!(all.contains(text::agent::MARKER_TREE_LEAF));
    assert!(all.contains(text::agent::TOKEN_IN));
    // 单套：不再有 a/b/c/d 候选标签。
    assert!(!all.contains("codex bullet tree"));
}

#[test]
fn render_does_not_panic_on_every_step_and_size() {
    for step in AgentStepId::ALL {
        for (width, height) in [(6, 4), (20, 6), (60, 50), (120, 60)] {
            let _ = render_step(step, width, height);
        }
    }
}

#[test]
fn every_step_renders_at_its_natural_height() {
    // 回归：单套样例的矮步骤（write/compact/mode 仅 2–3 行）曾因 `height < 4`
    // 的守卫被整体吞掉。这里用真实 `appearance_size` 给出的高度渲染，断言首行文本
    // （图标 + 动词）确实画出。
    let available = Rect::new(0, 0, 80, 200);
    for step in AgentStepId::ALL {
        let (width, height) = agent_appearance_size(step, available);
        let all = render_step(step, width, height);
        let first = step_lines(step).first().map(line_text).unwrap_or_default();
        assert!(
            !first.is_empty() && all.contains(&first),
            "步骤 {step:?}（宽 {width} 高 {height}）自然高度下未画出首行 {first:?}：\n{all}"
        );
    }
}

#[test]
fn render_is_borderless_and_does_not_use_panel_background() {
    // 无边框：不出现圆角/直线边框字符，且不铺浮层面板底色。
    let mut buf = Buffer::empty(Rect::new(0, 0, 60, 50));
    let area = buf.area;
    render_agent_appearance(&mut buf, area, AgentStepId::Exec);
    let all: String = (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    for border in ['╭', '╮', '╰', '╯', '┌', '┐'] {
        assert!(
            !all.contains(border),
            "简洁风不应出现边框字符 {border}: {all}"
        );
    }
    let panel_bg = style::overlay_panel().bg.expect("panel bg set");
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            assert_ne!(buf[(x, y)].bg, panel_bg, "不应铺面板底色");
        }
    }
}

#[test]
fn too_small_rect_renders_nothing() {
    let all = render_step(AgentStepId::Exec, 4, 2);
    assert_eq!(all.replace('\n', "").trim(), "", "过小区域不应绘制内容");
}

#[test]
fn agent_step_of_only_matches_agent_ids() {
    assert_eq!(
        agent_step_of(UiGalleryComponentId::Agent(AgentStepId::Edit)),
        Some(AgentStepId::Edit)
    );
    assert_eq!(agent_step_of(UiGalleryComponentId::Toast), None);
    assert_eq!(agent_step_of(UiGalleryComponentId::FolderSearch), None);
}

#[test]
fn icons_are_single_or_double_display_width() {
    use text::agent as a;
    let icons = [
        a::ICON_EXEC,
        a::ICON_READ,
        a::ICON_WRITE,
        a::ICON_EDIT,
        a::ICON_SEARCH,
        a::ICON_WEB,
        a::ICON_THINKING,
        a::ICON_SUBAGENT,
        a::ICON_TODO,
        a::ICON_MCP,
        a::ICON_COMPACT,
        a::ICON_MODE,
        a::ICON_FINAL,
        a::ICON_ERROR,
        a::ICON_QUESTION,
        a::ICON_TOKENS,
        a::MARKER_DOT,
        a::MARKER_TREE_LEAF,
        a::TODO_DONE,
        a::TODO_IN_PROGRESS,
        a::TODO_PENDING,
    ];
    for icon in icons {
        assert!(!icon.is_empty());
        let width: usize = icon.chars().map(|ch| char_width(ch) as usize).sum();
        assert!(
            width == 1 || width == 2,
            "图标 {icon:?} 宽度 {width} 非单/双列，跨平台不安全"
        );
    }
}
