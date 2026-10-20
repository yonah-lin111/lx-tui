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
///
/// **只服务 `[UI]` 查看器的 `agent` 分组样例**（`ui/overlay/ui_gallery/agent.rs`）。
/// 实时聊天历史的工具行不用它，改用 [`chat_tool_param`]：那边**刻意不加下划线**——
/// 连续多个工具行时下划线会把整块历史划成一片横线，盖过翡翠绿结构色
/// （见 [`chat_tool_param`] 与 `style::tests::agent_param_keeps_legacy_underline`）。
/// 二者同色（同为 [`CHAT_GOLD`]）而修饰符不同，是**有意为之**，不要「顺手对齐」。
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

// ===== 阅读渲染高亮（`ui::markdown_render` 专用）：GitHub Dark 官方色板 =====
//
// 只服务 lx 历史正文与 `[UI]` gallery 的 markdown **阅读渲染**。输入框的
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
// GitHub。固定 RGB 与 [`template_block_bg`] 同一条理由。
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

/// markdown 六级标题的样式：h1 加粗+下划线（对齐 GitHub 的 `border-bottom`）、h2 加粗+
/// 下划线、h3 加粗、h4–h6 斜体；全程只用修饰符，不换色（与 GitHub Dark 一致）。
pub fn md_read_heading_level(level: u8) -> Style {
    match level {
        1 => Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        2 => Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        3 => Style::default().add_modifier(Modifier::BOLD),
        _ => Style::default().add_modifier(Modifier::ITALIC),
    }
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

/// markdown 引用内容：GitHub 的引用只改前景 + 左侧竖条，不斜体、不换色。
pub fn md_read_quote() -> Style {
    Style::default().fg(github_dark::FG_MUTED)
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

/// markdown 代码块语言标记的 `language:` 前缀：实体紫 `#d2a8ff`（`--color-prettylights-syntax-entity`）。
///
/// 与语言名同色相、不加粗——`language:` 是标签、语言名是取值，靠字重拉开层级即可，
/// 不用两种颜色挤在一行里（此前的次要灰读起来像「注释」，起不到标示作用）。
pub fn md_read_code_lang() -> Style {
    Style::default().fg(github_dark::ENTITY)
}

/// markdown 代码块的**语言名**（`js` / `rust`…）：实体紫 `#d2a8ff`（`--color-prettylights-syntax-entity`）。
///
/// 与 `language:` 前缀同色相、靠**字重**分层级：前缀不加粗（标签）、语言名加粗（取值）。
/// 语言是这一行唯一有信息量的内容，紫色与代码 token（蓝/红/紫）同属语法色域，
/// 但用在该行的固定位置不会与正文混淆。
pub fn md_read_code_lang_name() -> Style {
    Style::default()
        .fg(github_dark::ENTITY)
        .add_modifier(Modifier::BOLD)
}

/// markdown 代码块的复制按钮 `[❐]`：与左侧容器折叠按钮同款（[`accent`]：Cyan 加粗）。
///
/// 全项目里「面板上的可点小控件」统一是这个青色加粗档（折叠按钮、面板按钮都用它），
/// 复制按钮跟着走，观感一致、也不用再引入一个新的交互色。
/// **不做悬停变色**：按钮只有 3 列，变色反而比常色更吵；可点性由 tooltip 承担。
pub fn md_read_code_copy() -> Style {
    accent()
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

/// 模板块标题占位符 `{title: …}`：Cyan 下划线（与状态色边框、@ 提及黄区分）。
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

/// 输入框上方临时提示的颜色：错误红 / 默认黄（用户指定）。
pub fn chat_input_hint(level: crate::app::state::InputHintLevel) -> Style {
    use crate::app::state::InputHintLevel;
    match level {
        InputHintLevel::Error => Style::default().fg(Color::Red),
        InputHintLevel::Warning => Style::default().fg(Color::Yellow),
    }
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

// ===== 聊天步骤 / 工具行的配色（翡翠绿结构色 + 金黄参数）=====
//
// lx 聊天历史里同时存在两类着色信息，必须一眼分得开：
//   A. **步骤结构**（方块、思考头、工具行）——本组 `chat_*` token，分三层：
//      ① 结构 / 状态（方块、头部文案、工具动词）：**翡翠绿 `Indexed 42`**（`#00d787`）；
//      ② 可变参数（路径 / 搜索范围）：金黄 `Indexed 220` + 下划线；
//      ③ 其余元信息（行范围、计数、状态文字）：`muted()`（终端 dim，无色相）。
//   B. **正文高亮**（markdown / 代码）——见上面的 `github_dark`：GitHub Dark 官方
//      色板，颜色只出现在链接（蓝）与代码 token（珊瑚红 / 浅蓝 / 蓝 / 紫 / 灰）上，
//      标题 / 粗体 / 斜体 / 引用全部是正文色 + 修饰符。
//
// **为什么不再是蓝**（第 5 轮，用户带截图反馈「与小方块与正文内容颜色特别相近」）：
// 上一版用 lx 页 `Native Agent` 标签里 `A` 的浅蓝 `Indexed 153`（`#afd7ff`），
// 用 CIEDE2000 量出来它和**正文**的色差只有 11.5、和 GitHub 的**字符串浅蓝**
// `#a5d6ff` 只有 2.1——等于方块、`Write`、`Thought` 与正文/代码字符串用了同一个颜色，
// 所以视觉上糊成一片。GitHub Dark 的 6 个代码 token 加上金黄参数已经把
// 蓝（207°/208°/212°/214°）、紫（269°）、珊瑚红（4°）、金（51°）全部占满，
// 256 色里唯一还剩 ≥30 色差余量的空闲色相是**绿 / 青绿（120°–170°）**。
// 因此结构色定在 `Indexed 42`（`#00d787`，158°）：与正文色差 30.1、与全部 GitHub
// 代码色最近 32.8、与金黄参数 36.2——三项同时 ≥30，是这张表里唯一达标的槽位。
// 备选（同样达标，偏好更亮可换）：`Indexed 49` `#00ffaf`（对深底亮度比 14.3 vs 10.0）。
//
// 语义：**热 = 进行中（方块加粗 + 呼吸）、冷 = 已完成（方块不加粗、静止）**；
// 失败红、中断黄、取消灰沿用既有状态色（语义优先于品牌色）。`[UI]` 查看器的
// agent 样例仍用上面的 `agent_*` 系列，不受这里影响。

/// 工具结构色：翡翠绿 `Indexed 42`（`#00d787`）。选色依据见上方注释的 CIEDE2000 数据。
const CHAT_STEP_GREEN: u8 = 42;
/// 思考结构色：品红 `Indexed 218`（`#ffafd7`）——与工具绿、参数金、正文默认色的
/// 色差分别为 68 / 55 / 24（CIEDE2000），一眼可分。
const CHAT_STEP_MAGENTA: u8 = 218;
/// 可变参数色：金黄（与 `[UI]` 查看器的 `agent_param` 同色；工具行不加下划线）。
const CHAT_GOLD: u8 = 220;
/// 工具**运行中**色：橙黄 `Indexed 208`（`#ff8700`）。
///
/// 语义是「还在进行」：**所有**正在运行的工具（文件工具、`exec_command`、
/// `write_stdin`、`update_plan`）的方块与动词统一用它，只有 `Completed` 才回翡翠绿。
/// 用户要求「橙黄一点」——比金黄参数 `220` 更橙更饱和（色差 27.9），比深土黄亮一档
/// （L 68.5 vs 58.5）；与工具绿 56.0、品红思考 40.7、天蓝 117 50.9 均 ≥40。
const CHAT_STEP_AMBER: u8 = 208;

// ----- 三类的方块 / 文案色（正文 = 默认前景，思考 = 品红，工具 = 翡翠绿）-----
//
// 用户要求：「正文内容的小方块用默认字体的颜色即可；thinking 的小方块和思考的字体
// 颜色使用其他的颜色」。因此**颜色由步骤类别决定**：
//   Body     → 终端默认前景（不设 fg）+ 加粗/不加粗区分运行态；
//   Thinking → 品红 218；
//   Tool     → 翡翠绿 42（含工具动词）。

/// 正文块的方块：运行中（呼吸）——**终端默认前景 + 加粗**，与正文本就同色。
pub fn chat_step_mark_running_body() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

/// 正文块的方块：已完成——终端默认前景、不加粗（与运行中的加粗呼吸方块区分）。
pub fn chat_step_mark_done_body() -> Style {
    Style::default()
}

/// 正文块的头部文案：运行中——默认前景、不加粗。
pub fn chat_step_label_running_body() -> Style {
    Style::default()
}

/// 正文块的头部文案：已完成——默认前景、不加粗。
pub fn chat_step_label_done_body() -> Style {
    Style::default()
}

/// 思考块的方块：运行中（呼吸）——品红加粗。
pub fn chat_step_mark_running_thinking() -> Style {
    Style::default()
        .fg(Color::Indexed(CHAT_STEP_MAGENTA))
        .add_modifier(Modifier::BOLD)
}

/// 思考块的方块：已完成——同款品红、不加粗（静止 + 细一档）。
pub fn chat_step_mark_done_thinking() -> Style {
    Style::default().fg(Color::Indexed(CHAT_STEP_MAGENTA))
}

/// 思考块的头部文案：运行中——品红（与方块同色系，不加粗）。
pub fn chat_step_label_running_thinking() -> Style {
    Style::default().fg(Color::Indexed(CHAT_STEP_MAGENTA))
}

/// 思考块的头部文案：已完成——品红（同上，不加粗）。
pub fn chat_step_label_done_thinking() -> Style {
    Style::default().fg(Color::Indexed(CHAT_STEP_MAGENTA))
}

/// 工具行的方块：运行中（呼吸）——翡翠绿加粗，整份历史里最亮的动作信号。
pub fn chat_step_mark_running() -> Style {
    Style::default()
        .fg(Color::Indexed(CHAT_STEP_GREEN))
        .add_modifier(Modifier::BOLD)
}

/// 工具行的方块：已完成——同款翡翠绿、**不加粗**（静止 + 细一档，与运行中区分）。
pub fn chat_step_mark_done() -> Style {
    Style::default().fg(Color::Indexed(CHAT_STEP_GREEN))
}

/// 工具**运行中**的方块：橙黄加粗（呼吸 + 加粗是唯一的进行中信号）。
///
/// 用户要求「所有正在运行中的 tool 的方块和 title 统一为黄色，完成才显示绿色」，
/// 因此不再按工具种类分色：文件工具、进程卡片、计划卡片共用这一档。
pub fn chat_step_mark_alive() -> Style {
    Style::default()
        .fg(Color::Indexed(CHAT_STEP_AMBER))
        .add_modifier(Modifier::BOLD)
}

/// 工具**运行中**的动词与状态文字：同档橙黄、不加粗（方块靠字重衬出来）。
pub fn chat_step_label_alive() -> Style {
    Style::default().fg(Color::Indexed(CHAT_STEP_AMBER))
}

/// 工具行的头部文案：运行中——橙黄（与方块同色系，不加粗）。
pub fn chat_step_label_running() -> Style {
    Style::default().fg(Color::Indexed(CHAT_STEP_AMBER))
}

/// 工具行的头部文案：已完成——翡翠绿（同上，不加粗）。
pub fn chat_step_label_done() -> Style {
    Style::default().fg(Color::Indexed(CHAT_STEP_GREEN))
}

/// 工具行的固定动词（`Read` / `Search` / `Edit` / `Write`）：**已完成**——翡翠绿、
/// 不加粗（与紧邻的加粗方块靠字重区分开）。
pub fn chat_tool_verb() -> Style {
    Style::default().fg(Color::Indexed(CHAT_STEP_GREEN))
}

/// 工具行的固定动词：**运行中**——与方块同档橙黄。
///
/// 用户要求「方块颜色和工具 title 统一为黄色」：动词是卡片的 title 主体，
/// 必须跟方块同色，否则一张卡片两种颜色。
pub fn chat_tool_verb_running() -> Style {
    Style::default().fg(Color::Indexed(CHAT_STEP_AMBER))
}

/// 工具行的可变参数（路径 / 搜索模式 / 搜索范围）：金黄，**不加下划线**。
///
/// 下划线在连续多个工具行时会把整块历史划成一片横线，反而盖过翡翠绿的结构色；
/// 去掉后靠金黄本身与正文/动词区分（与绿的结构色差 36，与最近的 GitHub token
/// 注释灰 42 / 关键字珊瑚 46 也拉得开）。
pub fn chat_tool_param() -> Style {
    Style::default().fg(Color::Indexed(CHAT_GOLD))
}

/// 工具行的只读元信息（`read` 的行范围）：退回 `muted()`（终端 dim、无色相）——
/// 它只是给主行做注脚，不值得再占一个色相档；显式前景会让它和动词抢注意力。
pub fn chat_tool_range() -> Style {
    muted()
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
