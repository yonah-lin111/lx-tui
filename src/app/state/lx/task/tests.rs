//! 后台任务页的状态模型：tab 轮转、高亮移动、列表替换与条目移除。

use crate::app::state::lx::agent::AgentsState;

use super::*;

fn entry(session_id: u64) -> ProcessEntry {
    ProcessEntry {
        session_id,
        command: format!("cmd-{session_id}"),
        cwd: "/workspace".into(),
        tty: false,
        started_at_ms: 0,
    }
}

fn page_with(count: u64) -> TaskPage {
    let mut page = TaskPage::default();
    let entries = (1..=count).map(entry).collect();
    assert!(page.set_processes(entries, 0));
    page
}

#[test]
fn new_page_starts_on_the_processes_tab() {
    let page = TaskPage::default();
    assert_eq!(page.tab, TaskTab::Processes);
    assert!(page.is_empty(&AgentsState::default()), "初始没有进程");
    assert_eq!(page.len(&AgentsState::default()), 0);
}

#[test]
fn cycle_tab_wraps_both_directions() {
    let mut page = page_with(2);
    assert!(page.cycle_tab(1));
    assert_eq!(page.tab, TaskTab::Agents);
    // 再向右回到第一个：两个 tab 之间循环。
    assert!(page.cycle_tab(1));
    assert_eq!(page.tab, TaskTab::Processes);
    assert!(page.cycle_tab(-1));
    assert_eq!(page.tab, TaskTab::Agents);
}

#[test]
fn agents_tab_is_empty_and_has_no_selection() {
    // 子代理 tab 本阶段是预留位：没有任何数据来源，长度恒为 0。
    let mut page = page_with(3);
    page.tab = TaskTab::Agents;
    assert_eq!(page.len(&AgentsState::default()), 0);
    assert!(page.is_empty(&AgentsState::default()));
    assert!(page.selected().is_none(), "预留 tab 没有可选项");
}

#[test]
fn selected_only_returns_processes_on_the_processes_tab() {
    let mut page = page_with(3);
    assert_eq!(page.selected().map(|p| p.session_id), Some(1));
    // 高亮在 Agents tab 上时即使进程列表非空也不选中。
    page.tab = TaskTab::Agents;
    assert!(page.selected().is_none());
}

#[test]
fn move_active_stops_at_both_ends_and_reports_whether_it_moved() {
    // **不循环**是「↑ 在顶部回到输入框」的前提：面板要能分辨"上移成功"与
    // "已经在顶部、无处可去"，后者才该退出面板。
    let mut page = page_with(3);
    assert!(!page.move_active(&AgentsState::default(), -1), "已在首项：上移不动，返回 false");
    assert_eq!(page.window.active, 0);
    assert!(page.move_active(&AgentsState::default(), 1), "下移成功");
    assert_eq!(page.window.active, 1);
    assert!(page.move_active(&AgentsState::default(), 1), "再下移成功");
    assert_eq!(page.window.active, 2, "钳在末项");
    assert!(!page.move_active(&AgentsState::default(), 1), "已在末项：下移不动，返回 false");

    let mut empty = TaskPage::default();
    assert!(!empty.move_active(&AgentsState::default(), 1), "空列表不移动");
    assert!(!empty.move_active(&AgentsState::default(), -1), "空列表不移动");
}

#[test]
fn set_processes_clamps_the_highlight_when_the_list_shrinks() {
    let mut page = page_with(3);
    page.window.move_active(2, 3);
    assert_eq!(page.window.active, 2);
    assert!(page.set_processes(vec![entry(1)], 0));
    assert_eq!(page.window.active, 0, "高亮被钳到唯一一项");
}

#[test]
fn set_processes_reports_no_change_for_identical_input() {
    let mut page = page_with(2);
    let same = page.processes.clone();
    assert!(!page.set_processes(same, page.now_ms), "内容与时间戳都没变");
}

#[test]
fn retain_process_removes_the_entry_and_clamps() {
    let mut page = page_with(3);
    page.window.move_active(2, 3);
    assert!(page.retain_process(3));
    assert_eq!(page.processes.len(), 2);
    assert_eq!(page.window.active, 1, "高亮被钳回有效范围");
    // 已移除的 id 再删一次没有变化。
    assert!(!page.retain_process(3));
}

#[test]
fn age_label_uses_seconds_minutes_and_hours() {
    let process = ProcessEntry {
        session_id: 1,
        command: "x".into(),
        cwd: "/".into(),
        tty: false,
        started_at_ms: 1_000,
    };
    assert_eq!(process.age_label(1_000 + 12_000), "12s");
    assert_eq!(process.age_label(1_000 + 90_000), "1m 30s");
    assert_eq!(process.age_label(1_000 + 7_500_000), "2h 05m");
    // 时钟回拨（now < started）按 0 计，不出现负数时长。
    assert_eq!(process.age_label(0), "0s");
}

#[test]
fn tab_labels_cover_every_tab() {
    assert_eq!(TaskTab::ALL.len(), 2);
    for tab in TaskTab::ALL {
        assert!(!tab.label().is_empty(), "{tab:?} 缺少标签");
    }
}
