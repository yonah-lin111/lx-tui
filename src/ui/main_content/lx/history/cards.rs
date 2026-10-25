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
/// `card` 是点击面：命令卡片的参数区与输出区各自登记一条，点其中一块只切换那一块；
/// 文件工具是整块一个点击面。折叠内容不再随鼠标悬停提亮（只有思考块保留悬停态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardHit {
    pub kind: CardKind,
    /// 点击面：命令卡片是单个分区（参数 / 输出 / 运行中整卡），文件工具是整块。
    pub card: Rect,
    /// 点击是否真的会改变内容（`false` 表示点了没反应，不登记点击面）。
    pub clickable: bool,
}
