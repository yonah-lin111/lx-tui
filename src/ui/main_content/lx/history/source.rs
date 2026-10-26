//! 历史渲染的只读数据源。
//!
//! 主窗格从 `&Lx` 构造，子 agent 详情从它自己的 `LxChat` + `LxCompact` 构造。
//! 渲染层只读：不克隆 `LxChat`、不访问 DB、不做 IO。

use crate::app::state::RewindNoticeView;
use crate::app::state::{ChatMessageView, CompactNoticeView, Lx, LxChat, LxCompact};

/// 历史渲染的只读数据源。
#[derive(Debug, Clone, Copy)]
pub(crate) struct HistorySource<'a> {
    /// 消息时间线。
    pub messages: &'a [ChatMessageView],
    /// 独立撤销提示。
    pub notices: &'a [RewindNoticeView],
    /// compact 检查点。
    pub checkpoints: &'a [CompactNoticeView],
    /// 折叠状态 / loading 相位。
    pub chat: &'a LxChat,
    /// compact 状态。
    pub compact: &'a LxCompact,
    /// 撤销面板当前选中的用户消息（渲染高亮用；无则 None）。
    pub selected_user_message: Option<&'a str>,
}

impl<'a> From<&'a Lx> for HistorySource<'a> {
    fn from(lx: &'a Lx) -> Self {
        Self {
            messages: &lx.chat.messages,
            notices: &lx.chat.notices,
            checkpoints: &lx.compact.checkpoints,
            chat: &lx.chat,
            compact: &lx.compact,
            selected_user_message: lx
                .rewind
                .selected
                .as_ref()
                .map(|target| target.user_message_id.as_str()),
        }
    }
}

impl<'a> HistorySource<'a> {
    /// 只有一份 `LxChat` 的数据源（`Agents` 详情区）。
    pub(crate) fn from_chat(chat: &'a LxChat, compact: &'a LxCompact) -> HistorySource<'a> {
        HistorySource {
            messages: &chat.messages,
            notices: &chat.notices,
            checkpoints: &compact.checkpoints,
            chat,
            compact,
            selected_user_message: None,
        }
    }

    /// 是否有可绘制的历史。
    pub(crate) fn has_history(&self) -> bool {
        !self.messages.is_empty() || !self.notices.is_empty() || !self.compact.is_empty()
    }
}
