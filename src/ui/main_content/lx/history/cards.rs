//! 聊天历史里可点击块的类型定义（布局产出、命中消费）。

use ratatui::layout::Rect;

use crate::app::state::CardSection;

/// 可交互块：折叠卡片或 agent 链接。
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
    /// 模型切换提示条目：整块可点击展开/折叠详情与注入提示词。
    ModelSwitch { switch_id: String, expanded: bool },
    /// 消息末尾的预览图片附件文件名（点击在系统文件管理器中定位该文件）。
    Attachment { path: String },
    /// 助手生成图片右侧的原图下载按钮（点击下载原图或定位已下载原图）。
    DownloadOriginalImage { attachment_id: String },
    /// 消息尾部 recap 系统事件卡（按消息 ID 定位）。
    Recap { message_id: String },
    /// 子 agent 汇报块：整块可点击展开/折叠正文。
    Report {
        message_id: String,
        agent_id: String,
        expanded: bool,
        /// 是否是 `… N more lines` 提示行（点击追加一批，而不是折叠整块）。
        more: bool,
    },
    /// 子 agent 卡片内的 agent 链接（不是折叠块）：点击打开 Agents 面板。
    AgentLink { agent_id: String },
    /// 用户消息正文折叠块（按稳定消息 ID 定位）。
    UserMessage { message_id: String, expanded: bool },
    /// 用户消息内的折叠粘贴芯片（按消息 ID + 粘贴块哈希 + 出现序号定位）。
    UserPaste {
        message_id: String,
        paste_id: u64,
        occurrence: usize,
        expanded: bool,
    },
}

/// 一个点击面的列范围口径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CardCols {
    /// 整行宽：运行中的命令/本地命令卡（打开后台任务面板）、agent 链接行。
    Full,
    /// 行内芯片：首行起始列、末行结束列；沿用用户粘贴芯片的多行口径。
    Chip { first: u16, last: u16 },
    /// 逐物理行的文字列区间（相对内容区左缘）；`None` 行不可点。
    Text(Vec<Option<(u16, u16)>>),
}

impl CardCols {
    /// 合并组调用体：去掉标题行后的区域切片。
    pub(crate) fn skip_rows(&self, skip: usize) -> Self {
        match self {
            Self::Full => Self::Full,
            Self::Chip { .. } => {
                // 芯片只出现在用户消息，不会进合并组；真要进来必须先补逐行切片。
                debug_assert_eq!(skip, 0, "Chip regions are never grouped");
                self.clone()
            }
            Self::Text(cols) => Self::Text(cols.get(skip..).unwrap_or(&[]).to_vec()),
        }
    }

    /// 仅用于缓存 LRU 记账的近似字节数。
    pub(crate) fn heap_bytes(&self) -> usize {
        match self {
            Self::Text(cols) => cols.len() * std::mem::size_of::<Option<(u16, u16)>>(),
            Self::Full | Self::Chip { .. } => 0,
        }
    }

    /// 逐行文字列里没有任何可点文字：展开态的纯空白块会走到这里。
    ///
    /// 这种块没有文字可点，若继续按列拒绝，鼠标就再也收不回去；命中侧
    /// 退化回整块（只在该块本来就 `clickable` 时生效）。
    pub(crate) fn text_is_blank(&self) -> bool {
        matches!(self, Self::Text(cols) if cols.iter().all(Option::is_none))
    }
}

/// 一个可折叠块的命中结果。
///
/// `card` 是本次命中对应的屏幕矩形：逐行文字块只覆盖被点那一行，芯片覆盖芯片
/// 列区间，整行块覆盖块矩形。折叠内容不再随鼠标悬停提亮（只有思考块保留悬停态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardHit {
    pub kind: CardKind,
    /// 本次命中的矩形（文字块为单行；整行块/芯片为块级裁剪）。
    pub card: Rect,
    /// 点击是否真的会改变内容（`false` 表示点了没反应，不登记点击面）。
    pub clickable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_skip_rows_drops_the_header() {
        let cols = CardCols::Text(vec![Some((0, 1)), None, Some((2, 3))]);
        assert_eq!(cols.skip_rows(1), CardCols::Text(vec![None, Some((2, 3))]));
        assert_eq!(cols.skip_rows(9), CardCols::Text(Vec::new()));
    }

    #[test]
    fn text_is_blank_only_when_every_row_is_none() {
        assert!(CardCols::Text(vec![None, None]).text_is_blank());
        assert!(!CardCols::Text(vec![None, Some((1, 2))]).text_is_blank());
        assert!(!CardCols::Full.text_is_blank());
    }

    #[test]
    fn full_and_chip_skip_keeps_their_own_semantics() {
        assert_eq!(CardCols::Full.skip_rows(3), CardCols::Full);
        let chip = CardCols::Chip { first: 1, last: 9 };
        assert_eq!(chip.skip_rows(0), chip);
    }
}
