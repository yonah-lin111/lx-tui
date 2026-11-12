//! 模型切换时间线条目的状态更新：列表回填与展开/折叠切换。

use crate::app::state::{AppState, ModelSwitchNoticeView};
use crate::db::chat::ModelSwitchRecord;
use crate::tiling::PaneId;

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
    let exists = state.pane_anywhere(pane).is_some_and(|target| {
        target
            .lx
            .model_switch
            .switches
            .iter()
            .any(|switch| switch.id == switch_id)
    });
    if !exists {
        return false;
    }
    // 展开会显示注入提示词并改变行数：跟随底部时先钉住当前阅读顶行。
    super::pin_lx_viewport(state, pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.model_switch.toggle(switch_id)
}
