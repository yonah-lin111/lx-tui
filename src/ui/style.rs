//! 语义样式 Token：沿用 codex 的终端原生 ANSI 配色。
//! 默认不铺自绘背景；标签栏的选中行使用 ANSI-16 背景（`selected_item`），
//! 悬停行使用明确的深色高亮背景（`selection`）；浮层面板使用自包含深底浅字（`overlay_panel`），
//! 浮层选中项使用显式高亮灰底白字（`overlay_selection`）。
//! 组件禁止直接写死颜色与修饰符。

use ratatui::style::{Color, Modifier, Style};

use crate::app::markdown::{SlashCommandId, TemplateStatus};
use crate::app::state::LxEntry;
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

/// 叠加在强调色边框上的灰字（显式前景 + dim）：临时提示词标题等。
///
/// `set_stringn` 只覆盖样式里显式给出的字段，`muted()` 不设前景，
/// 会把底层（聚焦态青色）边框前景带出来；这里显式重置前景。
pub fn muted_on_border() -> Style {
    Style::default()
        .fg(Color::Reset)
        .add_modifier(Modifier::DIM)
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

/// 边框上的正文（页重命名输入、页控件空白位）：显式重置前景，
/// 避免继承聚焦态边框的强调色；不带 dim，保证可编辑内容清晰。
pub fn text_on_border() -> Style {
    Style::default().fg(Color::Reset)
}

/// 主题强调色填充（Cyan 底、黑字、加粗）；激活标签使用。
pub fn selected_item() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

/// 列表/菜单当前项：高亮中灰底（Indexed 240）纯白字（Indexed 255）；侧栏悬停/拖拽与终端选区使用，与右键菜单 hover 对齐。
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

/// 临时提示词保存状态点：恒灰（草稿不参与自动保存）。
pub fn status_dot_draft() -> Style {
    Style::default().fg(Color::DarkGray)
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

/// Agent 步骤状态标记：成功绿实心、失败/中断红实心、进行中次要信息（与 `agent_status` 同色系）。
pub fn agent_ok() -> Style {
    Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD)
}

/// Agent 步骤失败/中断标记（Red 加粗）。
pub fn agent_fail() -> Style {
    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
}

/// Agent 通知的警告标记（Yellow 加粗）：本轮未完成等需要留意、但非失败的情况。
pub fn agent_warn() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

/// Agent 步骤标题左侧图标：lx 品牌粉（Indexed 218）加粗高亮，让图标从标题里跳出。
///
/// 失败步骤例外——整行走 [`agent_fail`] 变红。
pub fn agent_icon() -> Style {
    Style::default()
        .fg(Color::Indexed(218))
        .add_modifier(Modifier::BOLD)
}

/// Agent 步骤标题的固定动词（`Ran` / `Read` / `Edited`…）：lx 品牌天蓝（Indexed 117）。
///
/// 与可变参数、正文区分，且避开边框青（[`accent`]）与品牌粉（[`agent_icon`]）。
pub fn agent_verb() -> Style {
    Style::default().fg(Color::Indexed(117))
}

/// Agent 标题里可变参数的统一样式：固定金黄（Indexed 220）加下划线。
///
/// 命令 / 文件 / 目录 / agent / 查询 / 数值等**全部**用这一固定色（不再按类型分色），
/// 下划线提示“此处是每次调用都会变的值”。与图标（[`agent_icon`]）、动词
/// （[`agent_verb`]）都区分得开，且避开边框青。
pub fn agent_param() -> Style {
    Style::default()
        .fg(Color::Indexed(220))
        .add_modifier(Modifier::UNDERLINED)
}

/// Agent 标题里 read 的**行范围**（`(lines 1–2)`）：固定浅灰、无下划线。
///
/// 只读元信息，刻意与可变参数（金黄 + 下划线）区分：颜色不同、也不加下划线。
pub fn agent_range() -> Style {
    Style::default().fg(Color::Indexed(245))
}

/// Agent 步骤子行的直角连接符（`└─`）：品牌天蓝压暗，作结构引导而非内容。
pub fn agent_connector() -> Style {
    Style::default()
        .fg(Color::Indexed(117))
        .add_modifier(Modifier::DIM)
}

/// Agent 步骤子细节/输出：次要信息（dim 输出）。
pub fn agent_step_detail() -> Style {
    muted()
}

/// diff 新增行计数与前缀：Green。
pub fn agent_added() -> Style {
    Style::default().fg(Color::Green)
}

/// diff 删除行计数与前缀：Red。
pub fn agent_removed() -> Style {
    Style::default().fg(Color::Red)
}

/// token 页脚整行（lx-agent 的 `IN · OUT · CACHE`）：次要信息，不抢工具主体注意力。
pub fn agent_tokens() -> Style {
    muted()
}

/// 上下文占用 pill 的状态点：占用 ≥ 90% 红、≥ 75% 黄、否则绿（对齐 lx-agent）。
pub fn agent_context_dot(percent: u8) -> Style {
    let color = if percent >= 90 {
        Color::Red
    } else if percent >= 75 {
        Color::Yellow
    } else {
        Color::Green
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
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

/// 目录条目（`@` 提及面板与引用文件夹内容弹窗）：强调色，与文件条目一眼区分。
pub fn directory() -> Style {
    accent()
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

/// markdown 六级标题的样式：h1 加粗+下划线、h2 加粗、h3 加粗+斜体、h4–h6 斜体（对齐 codex）。
pub fn markdown_heading_level(level: u8) -> Style {
    let base = Color::Yellow;
    match level {
        1 => Style::default()
            .fg(base)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        2 => Style::default().fg(base).add_modifier(Modifier::BOLD),
        3 => Style::default()
            .fg(base)
            .add_modifier(Modifier::BOLD | Modifier::ITALIC),
        _ => Style::default().fg(base).add_modifier(Modifier::ITALIC),
    }
}

/// markdown 粗斜体（`***…***`）：Yellow 加粗+斜体。
pub fn markdown_strong_emphasis() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD | Modifier::ITALIC)
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

/// 代码高亮：关键字（`fn`/`const`/`class`/`return`…）——品牌品红加粗，最抢眼。
pub fn code_keyword() -> Style {
    Style::default()
        .fg(Color::Indexed(213))
        .add_modifier(Modifier::BOLD)
}

/// 代码高亮：字符串与字符字面量——绿。
pub fn code_string() -> Style {
    Style::default().fg(Color::Indexed(114))
}

/// 代码高亮：数字字面量——橙。
pub fn code_number() -> Style {
    Style::default().fg(Color::Indexed(215))
}

/// 代码高亮：注释——次要信息（dim）。
pub fn code_comment() -> Style {
    muted()
}

/// 代码高亮：类型名（首字母大写的标识符）——品牌天蓝。
pub fn code_type() -> Style {
    Style::default().fg(Color::Indexed(117))
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

/// markdown 无序列表项目符号：次要信息。
pub fn markdown_bullet() -> Style {
    muted()
}

/// markdown 有序列表序号（对齐 codex 的 light_blue）。
pub fn markdown_ordered() -> Style {
    Style::default().fg(Color::LightBlue)
}

/// markdown 任务列表已完成项（Green + 删除线）。
pub fn markdown_task_done() -> Style {
    Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::CROSSED_OUT)
}

/// markdown 任务列表未完成项：次要信息。
pub fn markdown_task_open() -> Style {
    muted()
}

/// markdown 水平线 / 表格边框：次要信息。
pub fn markdown_rule() -> Style {
    muted()
}

/// markdown 代码块语言标记：次要信息。
pub fn markdown_code_lang() -> Style {
    muted()
}

/// markdown 图片说明（alt / url）：次要信息。
pub fn markdown_image() -> Style {
    muted()
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

/// 模板块标题占位符 `{title: …}`：Cyan 下划线（与状态色边框、@ 提及黄区分）。
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

/// `@[refer-file](path)` 文件引用：LightGreen 下划线（与 @ 提及的 Yellow 区分）。
pub fn markdown_refer_file() -> Style {
    Style::default()
        .fg(Color::LightGreen)
        .add_modifier(Modifier::UNDERLINED)
}

/// `@[refer-folder](path)` 目录引用：橙（Indexed 208）下划线。
///
/// 固定 256 色橙：与 file 亮绿、image 亮品红、`@` 提及黄都能一眼分开，
/// 且不占用主题蓝/青（边框、URL、link 已用）。
pub fn markdown_refer_folder() -> Style {
    Style::default()
        .fg(Color::Indexed(208))
        .add_modifier(Modifier::UNDERLINED)
}

/// `@[refer-image](path)` 图片引用：LightMagenta 下划线。
pub fn markdown_refer_image() -> Style {
    Style::default()
        .fg(Color::LightMagenta)
        .add_modifier(Modifier::UNDERLINED)
}

/// `@ref/末两段` 短引用：柔紫（Indexed 141）加粗、无下划线。
///
/// 与既有引用色（绿 / 橙 / 亮品红）和 `@` 提及黄都不重样；`&&&` 命令名与 `+++` 边框用的是
/// 亮品红，141 明显更暗更柔，不会混淆。类型由形状表达（尾 `/` 文件夹、图片扩展名图片），
/// 因此不再按类型分色。
pub fn markdown_short_ref() -> Style {
    Style::default()
        .fg(Color::Indexed(141))
        .add_modifier(Modifier::BOLD)
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

/// 补充块 `+++` 结构标记：次要信息。
pub fn supple_marker() -> Style {
    muted()
}

/// 用户聊天消息整块底色（低饱和暗蓝，Indexed）：跨行一致、不抢正文。
pub fn chat_user_bg() -> Style {
    Style::default().bg(Color::Indexed(236))
}

/// 通知原因行的连接符（`└─`）：供应商同款灰色（[`chat_provider`]）+ 加粗。
pub fn chat_notice_connector() -> Style {
    chat_provider().add_modifier(Modifier::BOLD)
}

/// 状态行模型名：与 lx 页三个模式入口标签**同款**的粉→蓝逐字符渐变 + 加粗
/// （入口标签的 `selected` 态）。端点固定用 Native 的 `218→117`；`position`/`length`
/// 按**显示字符**计，渐变按整条标签取样，软换行不会改变某个字符的颜色。
pub fn model_label_style(position: usize, length: usize) -> Style {
    lx_entry_label_style(crate::app::state::LxEntry::Native, position, length, true)
}

/// 状态行供应商名：与本轮统计行（assistant 消息底部）同款次要信息色。
pub fn chat_provider() -> Style {
    muted()
}

/// lx 输入框上方 loading 行：转轮用品牌天蓝（Indexed 117）加粗。
pub fn chat_loading_spinner() -> Style {
    Style::default()
        .fg(Color::Indexed(117))
        .add_modifier(Modifier::BOLD)
}

/// lx 输入框上方 loading 行文案：次要信息。
pub fn chat_loading_label() -> Style {
    muted()
}

/// loading 行右端「回到底部」按钮：常态次要信息、悬停亮白。
pub fn chat_jump_bottom(hovered: bool) -> Style {
    if hovered {
        Style::default().fg(Color::Indexed(252))
    } else {
        muted()
    }
}

/// 补充块命令名：固定品红加粗（不随父 `&&&` 状态变化）。
pub fn supple_command() -> Style {
    Style::default()
        .fg(Color::LightMagenta)
        .add_modifier(Modifier::BOLD)
}

/// 补充块边框：固定品红（覆盖父块状态色）。
pub fn supple_border() -> Style {
    Style::default().fg(Color::LightMagenta)
}

/// 补充块整行底色：固定暗紫（覆盖父 `&&&` 状态底色）。
pub fn supple_block_bg() -> Style {
    Style::default().bg(Color::Rgb(46, 32, 54))
}

/// 块操作按钮（补充块）：常态次要信息。
pub fn block_button() -> Style {
    muted()
}

/// lx 页品牌 logo 像素色：按像素字符映射 256 色（仅 lx 页使用，不影响其余区域的 ANSI 槽位规则）。
///
/// `k` 描边藏青、`p` 主体粉、`n` 深粉、`b` 天空蓝、`c` 冰蓝高光、`w` 高光白；其余字符视为透明。
pub fn logo_pixel(ch: char) -> Option<Color> {
    match ch {
        'k' => Some(Color::Indexed(17)),
        'p' => Some(Color::Indexed(218)),
        'n' => Some(Color::Indexed(168)),
        'b' => Some(Color::Indexed(117)),
        'c' => Some(Color::Indexed(195)),
        'w' => Some(Color::Indexed(231)),
        _ => None,
    }
}

/// lx 页入口按钮的像素边框：选中用强调色、未选中用次要信息。
pub fn lx_entry_border(selected: bool) -> Style {
    if selected { accent() } else { muted() }
}

/// lx 页入口标签的渐变端点色（粉 → 蓝），取该入口 logo 的两主色（字标 L 侧粉、X 侧蓝）。
pub fn lx_entry_gradient(entry: LxEntry) -> (Color, Color) {
    match entry {
        LxEntry::Native => (Color::Indexed(218), Color::Indexed(117)),
        LxEntry::Hybrid => (Color::Indexed(168), Color::Indexed(195)),
        LxEntry::Automation => (Color::Indexed(195), Color::Indexed(168)),
    }
}

/// lx 页入口标签第 `position` 个字符（共 `length` 个）的样式：粉→蓝渐变取色，选中加粗。
pub fn lx_entry_label_style(
    entry: LxEntry,
    position: usize,
    length: usize,
    selected: bool,
) -> Style {
    let (from, to) = lx_entry_gradient(entry);
    let t = if length <= 1 {
        0.0
    } else {
        position as f32 / (length - 1) as f32
    };
    let base = Style::default().fg(lerp_indexed(from, to, t));
    if selected {
        base.add_modifier(Modifier::BOLD)
    } else {
        base
    }
}

/// 256 色 Indexed cube 段（16..=231）转 RGB；其它槽位返回 None。
fn cube_rgb(index: u8) -> Option<(u8, u8, u8)> {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let value = index.checked_sub(16)?;
    (value <= 215).then(|| {
        (
            LEVELS[usize::from(value / 36)],
            LEVELS[usize::from((value % 36) / 6)],
            LEVELS[usize::from(value % 6)],
        )
    })
}

/// RGB 归入最近的 256 色 cube 段（逐通道取最近色阶，平手取较暗者）。
fn cube_index((r, g, b): (u8, u8, u8)) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let nearest = |value: u8| {
        LEVELS
            .iter()
            .enumerate()
            .min_by_key(|(_, level)| level.abs_diff(value))
            .map_or(0, |(index, _)| index as u8)
    };
    16 + nearest(r) * 36 + nearest(g) * 6 + nearest(b)
}

/// 两个 256 色按 `t`∈[0,1] 线性插值，结果归入最近的 256 色。
fn lerp_indexed(from: Color, to: Color, t: f32) -> Color {
    let (Color::Indexed(a), Color::Indexed(b)) = (from, to) else {
        return if t < 0.5 { from } else { to };
    };
    let (Some(rgb_a), Some(rgb_b)) = (cube_rgb(a), cube_rgb(b)) else {
        return if t < 0.5 { from } else { to };
    };
    let mix = |x: u8, y: u8| {
        (f32::from(x) + (f32::from(y) - f32::from(x)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::Indexed(cube_index((
        mix(rgb_a.0, rgb_b.0),
        mix(rgb_a.1, rgb_b.1),
        mix(rgb_a.2, rgb_b.2),
    )))
}

/// lx 页输入框的像素边框：常态次要信息（焦点不改变输入框外观）。
pub fn lx_input_border() -> Style {
    muted()
}

#[cfg(test)]
mod tests;
