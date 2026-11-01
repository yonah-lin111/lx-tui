//! 模型切换时间线条目的状态更新：列表回填与展开/折叠切换。

use crate::app::state::{AppState, ModelSwitchNoticeView};
use crate::db::chat::ModelSwitchRecord;
use crate::layout::PaneId;

/// 替换窗格的模型切换记录列表（会话加载、切模型落库或撤销重载时调用）。
pub fn set_switches(state: &mut AppState, pane: PaneId, records: &[ModelSwitchRecord]) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let next: Vec<ModelSwitchNoticeView> = records
        .iter()
        .map(ModelSwitchNoticeView::from_record)
        .collect();
    if target.lx.model_switch.switches == next {
        return false;
    }
    target.lx.model_switch.replace(next);
    true
}

/// 切换一条模型切换提示的展开/折叠状态。
pub fn toggle_model_switch(state: &mut AppState, pane: PaneId, switch_id: &str) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.model_switch.toggle(switch_id)
}
