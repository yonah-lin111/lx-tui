//! 聊天历史里可点击块的类型定义（布局产出、命中消费）。

use ratatui::layout::Rect;

use crate::app::state::CardSection;

/// 可交互块：工具卡片或思考块。
///
/// `expanded` 是布局时算出的展开状态；折叠判定在鼠标松开时做，不在按下时做。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardKind {
    Tool {
        message_id: String,
        call_id: String,
        section: CardSection,
        expanded: bool,
    },
    Thinking {
        message_index: usize,
        step_index: usize,
        expanded: bool,
    },
    /// 本地命令卡片：命令行与输出各有自己的点击面。
    Shell {
        message_id: String,
        section: CardSection,
        expanded: bool,
    },
    /// compact 检查点条目：整块可点击展开/折叠摘要。
    Compact {
        checkpoint_id: String,
        expanded: bool,
    },
}

/// 一个可折叠块的矩形。
///
/// `card` 是点击面，`fold` 是 hover 面：命令卡片的参数区与输出区各自登记一条，
/// 点其中一块只切换那一块。文件工具仍是整块一个点击面、hover 只看折叠内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardHit {
    pub kind: CardKind,
    /// 点击面：命令卡片是单个分区（参数 / 输出 / 运行中整卡），文件工具是整块。
    pub card: Rect,
    /// **可折叠内容区**（标题之下、工具参数之上）：悬停提亮只跟这块走。
    ///
    /// 与 `card` 分开是因为两件事的判据不同：点击要覆盖整块（折叠态没有正文行，
    /// 只认内容区就点不开），而 hover 提亮只应该发生在**真的被折叠的内容**上——
    /// 展开后没有折叠内容，整块都不该亮；工具参数（详情行）在展开块之外，也不该亮。
    pub fold: Rect,
    /// 点击是否真的会改变内容：`false` 时只响应 hover（点了没反应比不亮更糟）。
    pub clickable: bool,
}
