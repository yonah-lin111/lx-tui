//! 语义样式 Token：沿用 codex 的终端原生 ANSI 配色。
//! 默认不铺自绘背景；侧栏/标签栏的选中行使用 ANSI-16 背景（`selected_item`），
//! 悬停行使用明确的深色高亮背景（`selection`）；浮层面板使用自包含深底浅字（`overlay_panel`），
//! 浮层选中项使用显式高亮灰底白字（`overlay_selection`）。
//! 组件禁止直接写死颜色与修饰符。

use ratatui::style::{Color, Modifier, Style};

use crate::app::markdown::{SlashCommandId, TemplateStatus};
use crate::detect::AgentState;

/// 正文。
pub fn text() -> Style {
    Style::default()
}

/// 次要信息：使用终端的 dim 修饰，自动适配明暗终端。
pub fn muted() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

/// 强调/激活：codex 在明暗未知终端上的 accent（Cyan 加粗）。
pub fn accent() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

/// 分区边框；焦点态使用 accent。
pub fn border(focused: bool) -> Style {
    if focused { accent() } else { muted() }
}

/// 面板边框标题：淡蓝色（Cyan + dim、不加粗），与分支状态同色系。
///
/// 标题样式叠加在边框样式之上，这里显式指定前景并移除粗体，
/// 保证焦点/浮层的强调色边框不会改变标题观感。
pub fn border_title() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::DIM)
        .remove_modifier(Modifier::BOLD)
}

/// 选中项：主题强调色填充（Cyan 底、黑字、加粗）；侧栏激活项与激活标签共用。
pub fn selected_item() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

/// 列表/菜单当前项：高亮中灰底（Indexed 240）纯白字（Indexed 255）；侧栏/标签栏的鼠标悬停与拖拽使用，与右键菜单 hover 对齐。
pub fn selection() -> Style {
    Style::default()
        .fg(Color::Indexed(255))
        .bg(Color::Indexed(240))
}

/// 浮层内选中/悬停项：在 overlay_panel（Indexed 236）底色之上使用更高亮的中灰背景（Indexed 240）与高亮白字（Indexed 255）。
pub fn overlay_selection() -> Style {
    Style::default()
        .fg(Color::Indexed(255))
        .bg(Color::Indexed(240))
}

/// 状态：失败（Red）。
pub fn error() -> Style {
    Style::default().fg(Color::Red)
}

/// prompt 保存状态点：已保存 Green、未保存 Yellow。
pub fn status_dot(saved: bool) -> Style {
    Style::default().fg(if saved { Color::Green } else { Color::Yellow })
}

/// 浮层标题：正文加粗。
pub fn strong() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

/// 边框标题内的次要值（prompt `ws:` 值、窗格 `agent:` 值）：
/// 聚焦面板的边框为强调色，值在其上叠加 dim；失焦面板回退 muted。
/// 与 ratatui 把标题 span 叠加到边框样式上的结果一致（聚焦值 = Cyan + BOLD + DIM）。
pub fn border_value(focused: bool) -> Style {
    if focused {
        accent().add_modifier(Modifier::DIM)
    } else {
        muted()
    }
}

/// Agent 状态圆点：Working 绿、Blocked 黄、Idle 次要信息（Unknown 同 Idle）。
pub fn agent_status(state: AgentState) -> Style {
    match state {
        AgentState::Working => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        AgentState::Blocked => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        AgentState::Idle | AgentState::Unknown => muted(),
    }
}

/// 浮层面板：自包含深底浅字（256 色），保证明暗终端下的对比度。
///
/// 用于 toast、右键菜单、命令面板与模态容器的整块着色：底色 Indexed 236、
/// 文字前景 Indexed 252；边框与强调/错误色在各自绘制时覆盖前景。
pub fn overlay_panel() -> Style {
    Style::default()
        .fg(Color::Indexed(252))
        .bg(Color::Indexed(236))
}

/// markdown 语法标记（`#`、`**`、`` ` `` 等）：次要信息。
pub fn markdown_marker() -> Style {
    muted()
}

/// markdown 标题文字。
pub fn markdown_heading() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

/// markdown 粗体文字。
pub fn markdown_strong() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

/// markdown 斜体文字。
pub fn markdown_emphasis() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::ITALIC)
}

/// markdown 删除线文字。
pub fn markdown_strikethrough() -> Style {
    Style::default()
        .fg(Color::LightRed)
        .add_modifier(Modifier::CROSSED_OUT)
}

/// markdown 行内代码内容。
pub fn markdown_inline_code() -> Style {
    Style::default().fg(Color::LightRed)
}

/// markdown 围栏代码块内容：不引入语法着色，保持正文色。
pub fn markdown_code_block() -> Style {
    text()
}

/// markdown 引用内容。
pub fn markdown_quote() -> Style {
    Style::default()
        .fg(Color::LightMagenta)
        .add_modifier(Modifier::ITALIC)
}

/// markdown 链接文字。
pub fn markdown_link_text() -> Style {
    Style::default()
        .fg(Color::LightBlue)
        .add_modifier(Modifier::UNDERLINED)
}

/// markdown URL。
pub fn markdown_url() -> Style {
    Style::default().fg(Color::Cyan)
}

/// 模板块结构标记（`&&&`、`--start`/`--end`、状态与元数据）：次要信息。
pub fn template_marker() -> Style {
    muted()
}

/// 模板块命令名：按业务分色并加粗；未知命令用通用紫。
pub fn template_command(id: Option<SlashCommandId>) -> Style {
    let color = match id {
        Some(SlashCommandId::Add) => Color::Green,
        Some(SlashCommandId::Bug) => Color::LightRed,
        Some(SlashCommandId::Refactor) => Color::LightMagenta,
        Some(SlashCommandId::Common) => Color::LightBlue,
        Some(SlashCommandId::Style) => Color::Magenta,
        None => Color::LightMagenta,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

/// 模板块标题占位符 `「title: …」`：Cyan 下划线（与状态色边框、@ 提及黄区分）。
pub fn template_title() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::UNDERLINED)
}

/// `@文件` 提及：Yellow 下划线。
pub fn markdown_file_mention() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::UNDERLINED)
}

/// 模板块操作按钮：常态次要信息。
pub fn template_button() -> Style {
    muted()
}

/// 模板块状态按钮：todo 次要、in_progress 黄、done 绿（对齐 lx-agent 状态色）。
pub fn template_status(status: TemplateStatus) -> Style {
    match status {
        TemplateStatus::Todo => muted(),
        TemplateStatus::InProgress => Style::default().fg(Color::Yellow),
        TemplateStatus::Done => Style::default().fg(Color::Green),
    }
}

/// 模板块边框：按状态分色（todo 靛蓝 / in_progress 黄 / done 绿）。
pub fn template_border(status: TemplateStatus) -> Style {
    match status {
        TemplateStatus::Todo => Style::default().fg(Color::LightBlue),
        TemplateStatus::InProgress => Style::default().fg(Color::Yellow),
        TemplateStatus::Done => Style::default().fg(Color::Green),
    }
}

/// 模板块整行底色：按状态铺低饱和暗色底（todo 暗灰蓝 / in_progress 暗灰琥珀 / done 暗灰绿）。
///
/// 固定 RGB，不受终端 256 色主题映射影响；只设置背景，前景与修饰符由各单元格自身样式决定。
pub fn template_block_bg(status: TemplateStatus) -> Style {
    let color = match status {
        TemplateStatus::Todo => Color::Rgb(35, 40, 56),
        TemplateStatus::InProgress => Color::Rgb(51, 41, 28),
        TemplateStatus::Done => Color::Rgb(29, 46, 36),
    };
    Style::default().bg(color)
}

/// lx 页吉祥物像素色：按像素字符映射 256 色（仅 lx 页使用，不影响其余区域的 ANSI 槽位规则）。
///
/// `k` 描边藏青、`p` 主体粉、`n` 深粉、`b` 天空蓝、`w` 高光白；其余字符视为透明。
pub fn mascot_pixel(ch: char) -> Option<Color> {
    match ch {
        'k' => Some(Color::Indexed(17)),
        'p' => Some(Color::Indexed(218)),
        'n' => Some(Color::Indexed(168)),
        'b' => Some(Color::Indexed(117)),
        'w' => Some(Color::Indexed(231)),
        _ => None,
    }
}

/// lx 页占位元素（白）：未来内容面板的边框与文案。
pub fn lx_placeholder() -> Style {
    Style::default().fg(Color::Indexed(231))
}

#[cfg(test)]
mod tests;
