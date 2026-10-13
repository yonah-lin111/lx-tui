//! 全局按钮 tooltip 模型：悬停停留后显示、目标变化重置计时、离开立即清除。
//!
//! 渲染锚点不存绝对坐标，由 ui 侧按当前几何重算；模型只持有目标与计时。

use std::time::{Duration, Instant};

use crate::layout::PaneId;

/// 悬停停留多久后显示。
pub const TOOLTIP_DELAY: Duration = Duration::from_millis(400);

/// 模板块按钮种类；与 ui 侧 `TemplateBlockButton` 一一对应，避免 app 依赖 ui。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateButtonKind {
    Status,
    Copy,
    Clean,
    Delete,
}

/// 可触发 tooltip 的按钮目标；文案映射由 `text::tooltip_hint` 穷尽匹配。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TooltipTarget {
    CollapseSidebar,
    CollapsePrompt,
    CollapseAgents,
    /// 窗格顶边框的视图切换按钮。
    PaneToggle(PaneId),
    AddWorkspace,
    AddTab,
    TabScrollLeft,
    TabScrollRight,
    Exit,
    /// 主区与 prompt 分隔线中点的交换按钮。
    SwapPromptSide,
    PromptMode,
    PromptTempPrompt,
    PromptUndo,
    PromptRedo,
    PromptSelectAll,
    PromptSave,
    PromptPin,
    /// 侧栏底边框左侧设置按钮。
    SidebarSettings,
    /// prompt 文本区内的模板块按钮；`line` 为块起始逻辑行。
    TemplateButton {
        line: usize,
        button: TemplateButtonKind,
    },
}

/// 鼠标当前悬停的按钮；到显示时刻前只计时不渲染。
#[derive(Debug)]
pub struct Tooltip {
    pub target: TooltipTarget,
    visible_at: Instant,
    visible: bool,
}

impl Tooltip {
    /// 从 `now` 起计时。
    pub fn new(target: TooltipTarget, now: Instant) -> Self {
        Self {
            target,
            visible_at: now + TOOLTIP_DELAY,
            visible: false,
        }
    }

    /// 是否已到显示时刻。
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// 到时提升为可见；返回是否发生变化。
    pub fn reveal(&mut self, now: Instant) -> bool {
        if self.visible || now < self.visible_at {
            return false;
        }
        self.visible = true;
        true
    }

    /// 下一次到期时间；已可见时不再有定时事件。
    pub fn next_deadline(&self) -> Option<Instant> {
        (!self.visible).then_some(self.visible_at)
    }
}

#[cfg(test)]
mod tests;
