//! 状态行测试：用量未知显示 `--`、大数缩写、窄窗格分行、停止/历史按钮几何。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::app::state::Lx;
use crate::app::state::lx::chat::ChatStatus;

fn status(
    model: Option<&str>,
    variant: Option<&str>,
    context: Option<u64>,
    input: Option<u64>,
    streaming: bool,
) -> ChatStatus {
    ChatStatus {
        model_label: model.map(str::to_string),
        variant_label: variant.map(str::to_string),
        context_limit: context,
        input_tokens: input,
        streaming,
        omitted_turns: None,
    }
}

fn lx_with_status(status: ChatStatus) -> Lx {
    let mut lx = Lx::new();
    lx.chat.status = status;
    lx
}

#[test]
fn unknown_tokens_show_dash_dash() {
    let st = status(Some("P / M"), None, None, None, false);
    assert_eq!(usage_text(&st), "--/--");
    let st = status(Some("P / M"), Some("high"), Some(128_000), None, false);
    assert_eq!(usage_text(&st), "--/128k");
    let st = status(Some("P / M"), Some("high"), None, Some(42_000), false);
    assert_eq!(usage_text(&st), "42k/--");
}

#[test]
fn format_tokens_abbreviates() {
    assert_eq!(format_tokens(0), "0");
    assert_eq!(format_tokens(999), "999");
    assert_eq!(format_tokens(1_000), "1k");
    assert_eq!(format_tokens(1_200), "1.2k");
    assert_eq!(format_tokens(42_000), "42k");
    assert_eq!(format_tokens(128_000), "128k");
    assert_eq!(format_tokens(1_000_000), "1m");
}

#[test]
fn wide_status_single_row_with_buttons() {
    let lx = lx_with_status(status(
        Some("Provider / Model"),
        Some("high"),
        Some(128_000),
        Some(42_000),
        true,
    ));
    let area = Rect::new(0, 0, 80, 1);
    let lay = layout(area, &lx.chat.status, rows(area, &lx.chat.status));
    assert_eq!(lay.area.height, 1);
    assert!(lay.stop.is_some(), "流中显示停止按钮");
    assert!(lay.history.is_some(), "历史入口常驻");
    // 按钮不重叠。
    let stop = lay.stop.unwrap();
    let history = lay.history.unwrap();
    assert!(stop.right() <= history.x, "停止按钮在历史按钮左侧");
}

#[test]
fn idle_status_hides_stop_button() {
    let lx = lx_with_status(status(Some("P / M"), None, Some(1000), None, false));
    let area = Rect::new(0, 0, 80, 1);
    let lay = layout(area, &lx.chat.status, rows(area, &lx.chat.status));
    assert!(lay.stop.is_none(), "空闲不显示停止按钮");
    assert!(lay.history.is_some());
}

#[test]
fn narrow_status_uses_two_rows() {
    let lx = lx_with_status(status(
        Some("Very Long Provider / Model"),
        Some("high"),
        Some(128_000),
        Some(42_000),
        false,
    ));
    let area = Rect::new(0, 0, 30, 2);
    assert_eq!(rows(area, &lx.chat.status), 2, "窄窗格两行");
    let lay = layout(area, &lx.chat.status, 2);
    assert_eq!(lay.area.height, 2);
    assert_eq!(lay.label.height, 2);
}

#[test]
fn zero_height_area_yields_no_status() {
    let lx = lx_with_status(status(Some("P / M"), None, None, None, false));
    let lay = layout(Rect::new(0, 0, 80, 0), &lx.chat.status, 0);
    assert_eq!(lay.area.height, 0);
    assert!(lay.history.is_none());
    assert!(lay.stop.is_none());
}

#[test]
fn render_writes_model_and_usage() {
    let lx = lx_with_status(status(
        Some("X / Y"),
        Some("low"),
        Some(128_000),
        Some(42_000),
        false,
    ));
    let area = Rect::new(0, 0, 60, 1);
    let mut grid = super::super::test_grid(area);
    let used = render(&mut grid, area, &lx);
    assert_eq!(used, 1);
    let mut buf = Buffer::empty(area);
    super::super::blit_grid(area, &mut buf, &grid);
    let text = (0..area.width)
        .filter_map(|x| buf.cell((x, 0)).map(|cell| cell.symbol().to_string()))
        .collect::<String>();
    assert!(text.contains("X / Y"), "显示模型：{text}");
    assert!(
        text.contains("high") || text.contains("low"),
        "显示等级：{text}"
    );
    assert!(text.contains("42k/128k"), "显示用量：{text}");
}

#[test]
fn omitted_notice_reserves_no_extra_row() {
    let mut st = status(Some("P / M"), None, Some(1000), None, false);
    st.omitted_turns = Some(3);
    let lx = lx_with_status(st);
    let area = Rect::new(0, 0, 80, 1);
    // 省略提示不改变行数（贴模型行右侧）。
    assert_eq!(rows(area, &lx.chat.status), 1);
}
