//! 语义样式 Token：沿用 codex 的终端原生 ANSI 配色。
//! 默认不铺自绘背景；标签栏的选中行使用 ANSI-16 背景（`selected_item`），
//! 悬停行使用明确的深色高亮背景（`selection`）；浮层面板使用自包含深底浅字（`overlay_panel`），
//! 浮层选中项使用显式高亮灰底白字（`overlay_selection`）。
//! 组件禁止直接写死颜色与修饰符。

use ratatui::style::{Color, Modifier, Style};

use crate::app::markdown::{SlashCommandId, TemplateStatus};
#[cfg(test)]
use crate::app::state::LxEntry;
use crate::detect::AgentState;

pub mod lx;
pub mod todo;

pub use lx::*;
pub use todo::*;

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

/// shell 模式前缀：浅红色，明确区别于聊天的 accent。
pub fn shell_prompt() -> Style {
    Style::default().fg(Color::LightRed)
}

/// 分区边框：恒为 muted（暗灰），不随焦点变色。
pub fn border(_focused: bool) -> Style {
    muted()
}

/// 叠加在强调色边框上的灰字（显式前景 + dim）：prompt 顶边框页名、页重命名 `· ` 分隔符、
/// 浮层底注等。
///
/// `set_stringn` 只覆盖样式里显式给出的字段，`muted()` 不设前景，
/// 会把底层（聚焦态青色）边框前景带出来；这里显式重置前景。
///
/// 同样必须显式移除 BOLD：该文字叠加在聚焦态的 `accent()` 边框上，
/// 否则会继承边框的粗体，聚焦时无故变粗一档。
pub fn muted_on_border() -> Style {
    Style::default()
        .fg(Color::Reset)
        .add_modifier(Modifier::DIM)
        .remove_modifier(Modifier::BOLD)
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

/// 标签内挂起 `ask` 的 `?` 标记：深琥珀/暗砖红（`Indexed 166` `#d75f00`，显式去掉继承的 DIM），背景跟随标签底色。
pub fn tab_ask_marker() -> Style {
    Style::default()
        .fg(Color::Indexed(166))
        .remove_modifier(Modifier::DIM)
}

/// 列表/菜单当前项：高亮中灰底（Indexed 240）纯白字（Indexed 255）；侧栏悬停/拖拽与终端选区使用，与右键菜单 hover 对齐。
pub fn selection() -> Style {
    Style::default()
        .fg(Color::Indexed(255))
        .bg(Color::Indexed(240))
}

/// 浮层内选中/悬停项：使用高亮中灰背景（Indexed 240）与高亮白字（Indexed 255）。
pub fn overlay_selection() -> Style {
    Style::default()
        .fg(Color::Indexed(255))
        .bg(Color::Indexed(240))
}

/// 模态主按钮（实心强调色胶囊块：Cyan 底、黑字、加粗；对齐 herdr 的 primary action button）。
pub fn button_primary() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

/// 模态危险确认主按钮（实心红底、亮白字、加粗；对齐 herdr 的 destructive action button）。
pub fn button_danger() -> Style {
    Style::default()
        .fg(Color::Indexed(255))
        .bg(Color::Red)
        .add_modifier(Modifier::BOLD)
}

/// 模态次级按钮（实心暗灰 surface 底、亮白字、加粗；对齐 herdr 的 surface0 action button）。
pub fn button_secondary() -> Style {
    Style::default()
        .fg(Color::Indexed(255))
        .bg(Color::Indexed(238))
        .add_modifier(Modifier::BOLD)
}

/// 模态单行输入框底条（实心暗色 surface 底、亮白字；对齐 herdr 的 surface0 输入条）。
pub fn modal_input() -> Style {
    Style::default()
        .fg(Color::Indexed(255))
        .bg(Color::Indexed(236))
}

/// 模态单行输入框占位文字（实心暗色 surface 底上的暗灰字）。
pub fn modal_input_placeholder() -> Style {
    Style::default()
        .fg(Color::Indexed(244))
        .bg(Color::Indexed(236))
}

/// 右键菜单项行内待确认态：深红底（Indexed 160）配亮白字（Indexed 255）加粗，对齐 lx-agent 的 dangerActive。
pub fn menu_confirm() -> Style {
    Style::default()
        .fg(Color::Indexed(255))
        .bg(Color::Indexed(160))
        .add_modifier(Modifier::BOLD)
}

/// Toast 提示框三档语义色（边框与右上角 `[✕]` 共用）：默认沿用浮层强调色，
/// warning 黄、danger 红——与模态弹窗/右键菜单同一套视觉语言（强调色边框）。
pub fn toast_border(kind: crate::app::toast::ToastKind) -> Style {
    use crate::app::toast::ToastKind;
    match kind {
        ToastKind::Default => accent(),
        ToastKind::Warning => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        ToastKind::Danger => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    }
}

/// Toast 标题色：与边框同色系，降为 dim 且不加粗（对齐浮层标题 `border_title`）。
pub fn toast_title(kind: crate::app::toast::ToastKind) -> Style {
    toast_border(kind)
        .remove_modifier(Modifier::BOLD)
        .add_modifier(Modifier::DIM)
}

/// 右侧容器顶边框标签页的激活项：深青蓝实心块（Indexed 24）+ 近白字 + 加粗。
///
/// 刻意**不复用** [`selected_item`]：那一套是顶部标签栏的"青底黑字"，
/// 同一屏上两处撞色会让人分不清哪一个是"当前工作区标签"、哪一个是"当前右侧面板"。
/// 这里用同色系（青）的深底并配亮字，既与容器边框呼应，又在亮度上明显区别于顶部标签栏。
pub fn right_tab_active() -> Style {
    Style::default()
        .fg(Color::Indexed(255))
        .bg(Color::Indexed(24))
        .add_modifier(Modifier::BOLD)
}

/// 右侧容器顶边框标签页的悬停项（未激活）：比激活块更暗一档的青灰底 + 常规字。
///
/// 悬停只是"指针在这里"，不该和激活同样醒目，因此不加粗。
pub fn right_tab_hover() -> Style {
    Style::default()
        .fg(Color::Indexed(252))
        .bg(Color::Indexed(237))
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

/// 边框上的标签前缀：窗格 `ws:`/`agent:`、prompt 底边框 `b:`/`wt:`/`ws:` 的前缀。
///
/// 淡青（Cyan + dim、**不加粗**），与 [`border_title`] 同色系，且失焦/聚焦完全一致：
/// 焦点只由边框框线颜色表达，前缀不应跟着变亮变粗。
///
/// BOLD 必须显式移除：这些前缀叠加写入边框单元格，
/// `Cell::set_style` 只覆盖样式里给出的字段，仅用 [`accent`] 会把边框的粗体一并带出来。
pub fn border_label_prefix() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::DIM)
        .remove_modifier(Modifier::BOLD)
}

/// 边框上的标签值：窗格 `ws:`/`agent:` 的值、prompt 底边框 `b:`/`wt:`/`ws:` 的值。
///
/// 恒为默认前景 + dim，**不随面板焦点变色**：聚焦只由边框框线颜色表达，
/// 标签值不应跟着变青（用户要求"移除聚焦变蓝、保留原本的默认颜色"）。
///
/// 前景与 BOLD 必须显式重置：这些文字直接叠加写入边框单元格，
/// `Cell::set_style` 只覆盖样式里给出的字段，仅用 [`muted`] 会把聚焦边框的
/// Cyan + BOLD 一并继承出来。
pub fn border_label_value() -> Style {
    Style::default()
        .fg(Color::Reset)
        .add_modifier(Modifier::DIM)
        .remove_modifier(Modifier::BOLD)
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

/// Agent 步骤子细节/输出：次要信息（dim 输出）。
pub fn agent_step_detail() -> Style {
    muted()
}

/// 折叠态“…”省略提示：与正文区分、统一斜体。
pub fn fold_hint() -> Style {
    muted().add_modifier(Modifier::ITALIC)
}

/// diff 新增行计数与前缀：Green。
pub fn agent_added() -> Style {
    Style::default().fg(Color::Green)
}

/// diff 删除行计数与前缀：Red。
pub fn agent_removed() -> Style {
    Style::default().fg(Color::Red)
}

/// diff 新增行的**整行底色**（深绿 `#213A2B`，取自 codex 的真彩兜底调色板）。
///
/// 与 [`agent_added`] 分工不同：前景色说明「这是新增」，底色说明「这一行的范围
/// 到哪」——只染前景时，空行与行尾的空白看不出属于哪一边。新增/删除行各自铺满
/// **整行宽度**，上下文行不铺（codex 同口径）。
pub fn agent_added_bg() -> Style {
    Style::default().bg(Color::Rgb(0x21, 0x3a, 0x2b))
}

/// diff 删除行的整行底色（深红 `#4A221D`，同为 codex 的真彩兜底值）。
pub fn agent_removed_bg() -> Style {
    Style::default().bg(Color::Rgb(0x4a, 0x22, 0x1d))
}

/// 浮层面板基础样式：不铺设背景色（配合 `clear` 清底保持终端原生透明底色），使用终端默认前景色。
pub fn overlay_panel() -> Style {
    Style::default()
}

/// 目录条目（`@` 提及面板与引用文件夹内容弹窗）：黄色，与文件条目一眼区分，不使用蓝色。
pub fn directory() -> Style {
    Style::default().fg(Color::Yellow)
}

// ===== 阅读渲染高亮（`ui::markdown_render` 专用）：GitHub Dark 官方色板 =====
//
// 只服务 lx 历史正文的 markdown **阅读渲染**。输入框的
// markdown **编辑高亮**是另一套独立色板（见下方 `markdown_*`），两者不得复用：
// 阅读侧要的是 GitHub 观感，编辑侧要的是打字时的语义分色。
//
// 用户要求「markdown 的格式高亮使用 GitHub 黑暗模式的」。此前两版都偏离目标：
// ① 沿袭 ANSI 语义色（黄标题 / 亮红行内代码 / 亮品红引用 / 亮蓝链接 / 五色代码）
//    —— 用户反馈“过于花里胡哨”；② 一刀切去色相（只剩默认前景 + 一档灰绿）
//    —— 强调全压在字重上，代码块失去语法信息。
//
// 现在直接**照抄 GitHub Dark 的两套官方 token**：
// - 语法高亮取 `github.github.com/_next/static/...` 的 prettylights 变量
//   （等价于 VS Code `GitHub Dark Default` 主题的 tokenColors）；
// - 文档结构取 `github-markdown-css` 的 dark 变量（正文 / 次要 / 边框 / 链接）。
//
// 用 `Color::Rgb` 而不是 256 色：GitHub 的十六进制值经过 256 色 cube 量化会明显偏色
// （例如 `#a5d6ff` 会落到 `#afd7ff`），而且用户主题对 Indexed 的重映射会让它不再是
// GitHub。
//
// GitHub Dark 的结构几乎**没有强调色**：标题/粗体/斜体/引用都只是正文色 + 修饰符，
// 颜色只出现在链接（`#4493f8`）与代码 token 上。因此本组不会重新变成“花里胡哨”。
mod github_dark {
    use ratatui::style::Color;

    /// 次要信息 `--color-fg-muted`（引用、URL、语法标记、表格/水平线）。
    ///
    /// 注：正文的 `--color-fg-default`（`#e6edf3`）**不落成样式**——正文继续用终端默认
    /// 前景，否则在浅色终端上会变成浅灰压白、不可读；GitHub 本身也只在自己的深底上用它。
    pub(super) const FG_MUTED: Color = Color::Rgb(0x7d, 0x85, 0x90);
    /// 链接 `--color-accent-fg`。
    pub(super) const LINK: Color = Color::Rgb(0x44, 0x93, 0xf8);
    /// 语法：关键字 / 类型声明 `--color-prettylights-syntax-keyword`。
    pub(super) const KEYWORD: Color = Color::Rgb(0xff, 0x7b, 0x72);
    /// 语法：字符串 `--color-prettylights-syntax-string`。
    pub(super) const STRING: Color = Color::Rgb(0xa5, 0xd6, 0xff);
    /// 语法：常量 / 数字 / 内联代码 `--color-prettylights-syntax-constant`
    /// （`markup.inline.raw` 同色）。
    pub(super) const CONSTANT: Color = Color::Rgb(0x79, 0xc0, 0xff);
    /// 语法：函数 / 实体 / 类型名 `--color-prettylights-syntax-entity`。
    pub(super) const ENTITY: Color = Color::Rgb(0xd2, 0xa8, 0xff);
    /// 语法：注释 `--color-prettylights-syntax-comment`。
    pub(super) const COMMENT: Color = Color::Rgb(0x8b, 0x94, 0x9e);
}

/// markdown 语法标记（`#`、`**`、`` ` `` 等）：GitHub 的次要前景色。
pub fn md_read_marker() -> Style {
    Style::default().fg(github_dark::FG_MUTED)
}

/// markdown 六级标题的统一样式：所有级别（h1–h6）统一为同一种格式，
/// 使用 GitHub Dark 实体紫 `#d2a8ff` + 加粗 + 下划线（无斜体）。
pub fn md_read_heading_level(_level: u8) -> Style {
    Style::default()
        .fg(github_dark::ENTITY)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

/// markdown 粗斜体（`***…***`）：正文色加粗+斜体（不再额外上色）。
pub fn md_read_strong_emphasis() -> Style {
    Style::default().add_modifier(Modifier::BOLD | Modifier::ITALIC)
}

/// markdown 粗体文字：正文色加粗。
pub fn md_read_strong() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

/// markdown 斜体文字：正文色斜体。
pub fn md_read_emphasis() -> Style {
    Style::default().add_modifier(Modifier::ITALIC)
}

/// markdown 删除线文字：GitHub 的 `markup.strikethrough` 只加删除线、不换色。
pub fn md_read_strikethrough() -> Style {
    Style::default().add_modifier(Modifier::CROSSED_OUT)
}

/// markdown 行内代码内容：GitHub 的 `markup.inline.raw` 用常量蓝 `#79c0ff`。
///
/// 按用户要求恢复为**纯前景色、不铺背景**（此前试过加暗色底，观感偏重）。
pub fn md_read_inline_code() -> Style {
    Style::default().fg(github_dark::CONSTANT)
}

/// markdown 围栏代码块内容：不引入语法着色，保持正文色。
pub fn md_read_code_block() -> Style {
    text()
}

/// 代码高亮：关键字（`fn`/`const`/`class`/`return`…）——
/// GitHub 的 `keyword` 珊瑚红 `#ff7b72`（不加粗：GitHub 自身也不加粗）。
pub fn code_keyword() -> Style {
    Style::default().fg(github_dark::KEYWORD)
}

/// 代码高亮：字符串与字符字面量——GitHub 的 `string` 浅蓝 `#a5d6ff`。
pub fn code_string() -> Style {
    Style::default().fg(github_dark::STRING)
}

/// 代码高亮：数字字面量——GitHub 的 `constant` 蓝 `#79c0ff`。
pub fn code_number() -> Style {
    Style::default().fg(github_dark::CONSTANT)
}

/// 代码高亮：类型名（首字母大写的标识符）——GitHub 的 `entity` 紫 `#d2a8ff`。
pub fn code_type() -> Style {
    Style::default().fg(github_dark::ENTITY)
}

/// 代码高亮：注释——GitHub 的 `comment` 灰 `#8b949e`。
pub fn code_comment() -> Style {
    Style::default().fg(github_dark::COMMENT)
}

/// markdown 引用内容：使用与底栏模型上下文占用（[`chat_provider`]）同款的次要信息灰色（`muted()`），
/// 避免使用 `FG_MUTED`（`#7d8590`）叠加 `DIM` 后变成过暗的死灰。
pub fn md_read_quote() -> Style {
    chat_provider()
}

/// markdown 链接文字：GitHub 的链接蓝 `#4493f8` + 下划线。
pub fn md_read_link_text() -> Style {
    Style::default()
        .fg(github_dark::LINK)
        .add_modifier(Modifier::UNDERLINED)
}

/// markdown URL：GitHub 把裸 URL 与链接地址都渲染成链接蓝。
pub fn md_read_url() -> Style {
    Style::default().fg(github_dark::LINK)
}

/// markdown 无序列表项目符号：GitHub 的列表符号与正文同色，但终端里同色会让
/// 列表结构看不出来，因此用链接蓝 `#4493f8` 作为**结构强调**（只染符号，不动正文）。
pub fn md_read_bullet() -> Style {
    Style::default().fg(github_dark::LINK)
}

/// markdown 有序列表序号：与无序项目符号同色，保持列表结构的一致观感。
pub fn md_read_ordered() -> Style {
    Style::default().fg(github_dark::LINK)
}

/// markdown 任务列表已完成项复选框 `[x]`：绿色（状态色，只染复选框）。
pub fn md_read_task_done() -> Style {
    Style::default().fg(Color::Green)
}

/// markdown 任务列表未完成项复选框 `[ ]`：黄色（待办，只染复选框）。
pub fn md_read_task_open() -> Style {
    Style::default().fg(Color::Yellow)
}

/// markdown 水平线 / 表格边框：GitHub 的边框色（`#30363d`）在终端里太暗，
/// 用 `--color-fg-muted` 档兼顾可见度。
pub fn md_read_rule() -> Style {
    Style::default().fg(github_dark::FG_MUTED)
}

/// markdown 图片说明（alt / url）：GitHub 的次要前景色。
pub fn md_read_image() -> Style {
    Style::default().fg(github_dark::FG_MUTED)
}

// ===== 编辑器 markdown 高亮（prompt 编辑器与 lx 输入框共用）：ANSI 语义色 =====
//
// **与上方 `md_read_*` 阅读色板完全独立**：这里是打字时的语法高亮，需要一眼分得清
// 标题 / 强调 / 行内代码 / 引用 / 链接，因此保留 ANSI 语义槽位（Yellow / LightRed /
// LightMagenta / LightBlue / Cyan）。改动阅读侧的 GitHub Dark 不会、也不应影响这里。
//
// 消费方：`ui::prompt::token_style`（prompt 编辑器 + lx 多行输入框）。
// `ui::markdown_render` 绝不使用本组函数。

/// markdown 语法标记（`#`、`**`、`` ` `` 等）：次要信息（dim）。
pub fn markdown_marker() -> Style {
    muted()
}

/// markdown 标题文字。
pub fn markdown_heading() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

/// markdown 标题的 `#` 标记：与标题同色相但压暗，让人看得出"这是标题符号"
/// 而不与正文的通用标记（dim 灰）混同。
pub fn markdown_heading_marker() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::DIM)
}

/// markdown 粗体文字：Yellow 加粗。
pub fn markdown_strong() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

/// markdown 斜体文字：Yellow 斜体。
pub fn markdown_emphasis() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::ITALIC)
}

/// markdown 删除线文字：亮红 + 删除线。
pub fn markdown_strikethrough() -> Style {
    Style::default()
        .fg(Color::LightRed)
        .add_modifier(Modifier::CROSSED_OUT)
}

/// markdown 行内代码内容：亮红（不铺背景，与阅读侧口径一致）。
pub fn markdown_inline_code() -> Style {
    Style::default().fg(Color::LightRed)
}

/// markdown 无序列表项目符号：Yellow 加粗（只染符号，正文保持原色）。
pub fn markdown_bullet() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

/// markdown 有序列表序号：与无序项目符号同款，保持列表结构的一致观感。
pub fn markdown_ordered() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

/// markdown 任务列表未完成复选框 `[ ]`：Yellow（待办）。
pub fn markdown_task_open() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

/// markdown 任务列表已完成复选框 `[x]`：Green（完成）。
pub fn markdown_task_done() -> Style {
    Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD)
}

/// markdown 围栏代码块内容：不引入语法着色，保持正文色。
pub fn markdown_code_block() -> Style {
    text()
}

/// 折叠粘贴芯片：按语义类型区分前景色与深色背景（Text 黄字深灰底、Code 青字深蓝底、Log 亮绿字深绿底）。
pub fn pasted_chip(kind: crate::app::paste::FoldedKind) -> Style {
    use crate::app::paste::FoldedKind;
    let (fg, bg) = match kind {
        FoldedKind::Text => (Color::Yellow, Color::Indexed(236)),
        FoldedKind::Code => (Color::Cyan, Color::Indexed(23)),
        FoldedKind::Log => (Color::LightGreen, Color::Indexed(22)),
    };
    Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD)
}

/// 折叠粘贴展开区：与对应类型芯片共用同一深色背景，正常前景。
pub fn pasted_bg(kind: crate::app::paste::FoldedKind) -> Style {
    use crate::app::paste::FoldedKind;
    let bg = match kind {
        FoldedKind::Text => Color::Indexed(236),
        FoldedKind::Code => Color::Indexed(23),
        FoldedKind::Log => Color::Indexed(22),
    };
    Style::default().bg(bg)
}

/// markdown 引用内容：亮品红斜体。
pub fn markdown_quote() -> Style {
    Style::default()
        .fg(Color::LightMagenta)
        .add_modifier(Modifier::ITALIC)
}

/// markdown 链接文字：亮蓝下划线。
pub fn markdown_link_text() -> Style {
    Style::default()
        .fg(Color::LightBlue)
        .add_modifier(Modifier::UNDERLINED)
}

/// markdown URL：青色。
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

/// 模板块标题占位符 `{title: …}`：Cyan 下划线（与状态标签色、@ 提及黄区分）。
pub fn template_title() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::UNDERLINED)
}

/// `@文件` 提及：Yellow 下划线（编辑器高亮，与阅读渲染的 `md_read_*` 无关）。
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

/// 用户消息下方的文件引用树分支标签色（无下划线）。
pub fn user_ref_file() -> Style {
    Style::default().fg(Color::LightGreen)
}

/// 用户消息下方的文件夹引用树分支标签色（无下划线）。
pub fn user_ref_folder() -> Style {
    Style::default().fg(Color::Indexed(208))
}

/// 用户消息下方的图片引用树分支标签色（无下划线）。
pub fn user_ref_image() -> Style {
    Style::default().fg(Color::LightMagenta)
}

/// `@[ref-content]/...` 快捷引用：柔紫（Indexed 141）加粗、无下划线。
pub fn markdown_ref_content() -> Style {
    Style::default()
        .fg(Color::Indexed(141))
        .add_modifier(Modifier::BOLD)
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

/// &&& 模板块色条与分割线：固定为 lx 品牌粉（`Indexed 218`），不随状态变色；
/// 与 +++（品红）及三态色（蓝/黄/绿）保持区分。
pub fn template_block_bar() -> Style {
    Style::default().fg(LX_PINK)
}

/// 标题行页名状态色（对齐 recent 页状态圆点：todo 默认亮字、in_progress 黄、done 绿）。
///
/// 叠加在 `DIM` 分割线上绘制，必须显式移除 `DIM` 与 `BOLD`。
pub fn template_border(status: TemplateStatus) -> Style {
    let fg = match status {
        TemplateStatus::Todo => Color::Reset,
        TemplateStatus::InProgress => Color::Yellow,
        TemplateStatus::Done => Color::Green,
    };
    Style::default()
        .fg(fg)
        .remove_modifier(Modifier::DIM)
        .remove_modifier(Modifier::BOLD)
}

/// 补充块 `+++` 结构标记：次要信息。
pub fn supple_marker() -> Style {
    muted()
}

/// 用户聊天消息整块底色（低饱和暗蓝，Indexed）：跨行一致、不抢正文。
pub fn chat_user_bg() -> Style {
    Style::default().bg(Color::Indexed(236))
}

/// 撤销选中的最新用户消息整块底色（深青蓝，与普通用户消息区分）。
pub fn chat_user_target_bg() -> Style {
    Style::default().bg(Color::Indexed(24))
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

/// 状态行思考等级（variant）：**加粗黄字**，对齐 opencode 的
/// `theme.text.feedback.warning.base` + `BOLD`。
///
/// 用终端原生 `Color::Yellow` 而非 RGB：本项目配色层只有 16 色语义 token，
/// 单独塞一个 RGB 琥珀会在不同终端主题下失控。
pub fn chat_variant() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

/// lx 输入框上方 loading 的**扫描高亮色**：按 `Native Agent` 标签的 4 档渐变循环
/// （`218` 粉 → `182` 淡紫 → `153` 浅蓝 → `117` 天蓝），加粗。
///
/// 每 `LOADING_FRAME_INTERVAL`（100ms）换一档，与扫描窗口同步推进——视觉上是
/// 颜色沿 `Working...` 从左到右流过。
pub fn chat_loading_scan(phase: usize) -> Style {
    let cycle = crate::ui::text::agent::native_label_cycle();
    let index = cycle.get(phase % cycle.len()).copied().unwrap_or(117);
    Style::default()
        .fg(Color::Indexed(index))
        .add_modifier(Modifier::BOLD)
}

#[cfg(test)]
mod tests;
