//! lx 输入框 `@` 提及状态单元测试：根同步、空查询、过滤、目录进入、采纳、撤销、
//! Esc 压制与过期结果丢弃。

use std::path::Path;

use super::*;
use crate::app::markdown::MentionEntry;
use crate::app::state::Lx;

/// 根路径常量；测试统一使用。
const ROOT: &str = "/work/demo";

/// 文件条目。
fn file(path: &str) -> MentionEntry {
    MentionEntry {
        path: path.into(),
        is_directory: false,
    }
}

/// 目录条目。
fn dir(path: &str) -> MentionEntry {
    MentionEntry {
        path: path.into(),
        is_directory: true,
    }
}

/// 一组演示候选：根级文件、目录与目录下的文件。
fn entries() -> Vec<MentionEntry> {
    vec![
        file("main.rs"),
        dir("src"),
        file("src/lib.rs"),
        file("src/app.rs"),
        file("README.md"),
    ]
}

/// 逐字符键入 `text`（触发面板重算），同步根并写入缓存候选。
fn ready(text: &str, entries: Vec<MentionEntry>) -> Lx {
    let mut lx = Lx::new();
    lx.set_mention_root(Some(std::path::PathBuf::from(ROOT)));
    for ch in text.chars() {
        lx.insert_char(ch);
    }
    if let Some((generation, _)) = lx.take_mention_scan_request() {
        lx.apply_mention_entries(generation, Path::new(ROOT), entries);
    }
    lx
}

#[test]
fn root_change_invalidates_cache_and_generation() {
    let mut lx = Lx::new();
    assert!(lx.set_mention_root(Some(std::path::PathBuf::from("/a"))));
    // 相同根不重复报告变化。
    assert!(!lx.set_mention_root(Some(std::path::PathBuf::from("/a"))));
    lx.insert_char('@');
    let (old_generation, old_root) = lx.take_mention_scan_request().expect("后有效 @ 查询");
    assert_eq!(old_root, std::path::PathBuf::from("/a"));
    // 根变化递增代号并作废在途结果。
    assert!(lx.set_mention_root(Some(std::path::PathBuf::from("/b"))));
    let (new_generation, new_root) = lx.take_mention_scan_request().expect("根变化后重扫");
    assert_eq!(new_root, std::path::PathBuf::from("/b"));
    assert_ne!(old_generation, new_generation, "根变化递增扫描代号");
    // 旧代号结果被丢弃，新代号结果采纳。
    assert!(!lx.apply_mention_entries(old_generation, Path::new("/b"), entries()));
    assert!(lx.apply_mention_entries(new_generation, Path::new("/b"), entries()));
}

#[test]
fn no_root_yields_no_scan_request() {
    let mut lx = Lx::new();
    lx.insert_char('@');
    assert!(lx.take_mention_scan_request().is_none(), "无根不发起扫描");
    assert_eq!(
        lx.mention().view(&lx.input, lx.cursor),
        LxMentionView::Hidden,
        "无根不显示面板"
    );
}

#[test]
fn empty_query_lists_all_candidates() {
    let lx = ready("@", entries());
    assert!(lx.mention_open());
    let paths: Vec<&str> = lx
        .mention()
        .panel()
        .expect("panel")
        .items()
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    assert_eq!(paths.len(), 5, "空查询列出根下全部候选: {paths:?}");
}

#[test]
fn typing_filters_candidates_without_rescan() {
    let mut lx = ready("@", entries());
    for ch in "lib".chars() {
        lx.insert_char(ch);
    }
    assert!(lx.take_mention_scan_request().is_none(), "有缓存不再重扫");
    let paths: Vec<String> = lx
        .mention()
        .panel()
        .expect("panel")
        .items()
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    assert_eq!(paths, vec!["src/lib.rs".to_string()]);
}

#[test]
fn enter_folder_scopes_candidates_and_is_single_undo_step() {
    let mut lx = ready("@s", entries());
    let panel = lx.mention().panel().expect("panel");
    assert!(
        panel.items()[panel.active()].is_directory,
        "高亮应为目录 src"
    );
    assert!(lx.mention_enter_folder(), "目录进入消费 Shift+Enter");
    assert_eq!(lx.input, "@src/");
    let mut paths: Vec<String> = lx
        .mention()
        .panel()
        .expect("panel scoped to src")
        .items()
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    paths.sort();
    assert_eq!(paths, vec!["src/app.rs", "src/lib.rs"], "只列目录内候选");
    // 目录进入是一步可撤销编辑。
    lx.undo();
    assert_eq!(lx.input, "@s", "撤销回到进入前");
}

#[test]
fn enter_folder_on_file_is_noop() {
    let mut lx = ready("@main", entries());
    let panel = lx.mention().panel().expect("panel");
    assert!(!panel.items()[panel.active()].is_directory);
    assert!(!lx.mention_enter_folder(), "文件不可进入");
    assert_eq!(lx.input, "@main", "文本不变");
}

#[test]
fn enter_folder_without_children_is_noop() {
    // 目录 `empty` 无缓存后代，不可进入。
    let mut lx = ready("@empty", vec![dir("empty")]);
    assert!(lx.mention().panel().is_some());
    assert!(!lx.mention_enter_folder(), "无子项目录不可进入");
    assert_eq!(lx.input, "@empty");
}

fn skill(name: &str, path: &str) -> crate::skills::SkillEntry {
    crate::skills::SkillEntry {
        name: name.into(),
        description: format!("{name} description"),
        path: std::path::PathBuf::from(path),
        scope: crate::skills::SkillScope::User,
        source: crate::skills::SkillSource::Lx,
        enabled: true,
        shadowed_by: None,
    }
}

#[test]
fn skill_and_mcp_candidates_switch_via_tabs_and_insert_references() {
    let mut lx = ready("@pd", entries());
    lx.set_mention_skills(vec![skill("pdf", "/skills/pdf/SKILL.md")]);
    lx.set_mention_mcp_servers(vec![McpMentionEntry::new(
        "codegraph",
        "stdio · codegraph serve --mcp",
    )]);
    // Files Tab 纯净展示文件/目录，`@pd` 在 Files 下无匹配。
    assert!(lx.mention().panel().is_none());
    assert_eq!(lx.mention_tab(), MentionTab::Files);
    assert!(lx.mention_visible());

    // 按右键两次：Files -> RefDir -> Skills
    assert!(lx.mention_switch_tab(1));
    assert_eq!(lx.mention_tab(), MentionTab::RefDir);
    assert!(lx.mention_switch_tab(1));
    assert_eq!(lx.mention_tab(), MentionTab::Skills);
    assert!(lx.mention_open());
    assert_eq!(lx.mention().filtered_skills().len(), 1);
    assert!(lx.mention_confirm());
    assert_eq!(lx.input, "@[refer-skill](/skills/pdf/SKILL.md) ");

    // 再次输入 `@code`，按左键直接切到最后一个 Tab（MCP），选中插入 `@[refer-mcp](codegraph) `
    for ch in "@code".chars() {
        lx.insert_char(ch);
    }
    assert!(lx.mention_switch_tab(-1));
    assert_eq!(lx.mention_tab(), MentionTab::Mcp);
    assert!(lx.mention_open());
    assert_eq!(lx.mention().filtered_mcp_servers().len(), 1);
    assert!(lx.mention_confirm());
    assert_eq!(
        lx.input,
        "@[refer-skill](/skills/pdf/SKILL.md) @[refer-mcp](codegraph) "
    );
    // 退格一次原子删除整块 `@[refer-mcp](codegraph) `
    lx.backspace();
    assert_eq!(lx.input, "@[refer-skill](/skills/pdf/SKILL.md) ");
}

#[test]
fn confirm_file_writes_mention_with_trailing_space() {
    let mut lx = ready("@main", entries());
    assert!(lx.mention_confirm());
    assert_eq!(lx.input, "@main.rs ", "文件插入相对路径带尾随空格");
    assert!(lx.mention().panel().is_none(), "确认后关闭面板");
    lx.undo();
    assert_eq!(lx.input, "@main", "采纳是一步可撤销编辑");
}

#[test]
fn confirm_directory_writes_trailing_slash_and_space() {
    let mut lx = ready("@sr", entries());
    assert!(lx.mention_confirm());
    assert_eq!(lx.input, "@src/ ", "目录插入带 / 与尾随空格");
}

#[test]
fn escape_keeps_query_and_suppresses_reopen() {
    let mut lx = ready("@ma", entries());
    assert!(lx.mention_open());
    assert!(lx.mention_escape());
    assert_eq!(lx.input, "@ma", "Esc 不删除已键入查询");
    assert!(!lx.mention_open(), "Esc 关闭面板");
    assert_eq!(
        lx.mention().view(&lx.input, lx.cursor),
        LxMentionView::Hidden,
        "Esc 后在未编辑前压制重开"
    );
    // 一次编辑解除压制并重新评估。
    lx.insert_char('i');
    assert!(lx.mention_open(), "编辑后重新评估");
}

#[test]
fn confirm_then_new_trigger_reopens() {
    let mut lx = ready("@main", entries());
    assert!(lx.mention_confirm());
    assert_eq!(lx.input, "@main.rs ");
    // 新的一行重新键入 @ 触发，面板重新打开。
    lx.insert_char('\n');
    for ch in "@ma".chars() {
        lx.insert_char(ch);
    }
    assert!(lx.mention_open(), "新触发重新打开面板");
}

#[test]
fn stale_generation_and_root_are_discarded() {
    let mut lx = Lx::new();
    lx.set_mention_root(Some(std::path::PathBuf::from("/root")));
    lx.insert_char('@');
    let (generation, _) = lx.take_mention_scan_request().expect("request");
    // 代号不匹配丢弃。
    assert!(!lx.apply_mention_entries(generation.wrapping_add(1), Path::new("/root"), entries()));
    // 根不匹配丢弃。
    assert!(!lx.apply_mention_entries(generation, Path::new("/other"), entries()));
    // 完全匹配采纳。
    assert!(lx.apply_mention_entries(generation, Path::new("/root"), entries()));
}

#[test]
fn scroll_viewport_only_moves_window_not_highlight() {
    let mut lx = ready("@", entries());
    let active = lx.mention().panel().expect("panel").active();
    assert!(lx.mention_scroll(2, 0));
    let panel = lx.mention().panel().expect("panel");
    assert_eq!(panel.viewport(), Some(2));
    assert_eq!(panel.active(), active, "滚轮不动高亮");
}

#[test]
fn mouse_select_confirms_active_candidate() {
    let mut lx = ready("@", entries());
    // 空查询按路径排序：README.md、main.rs、src、src/app.rs、src/lib.rs。
    // 第 3 项（索引 2）为目录 `src`。
    assert!(lx.mention_set_active(2));
    assert_eq!(lx.mention().panel().expect("panel").active(), 2);
    assert!(lx.mention_confirm());
    assert_eq!(lx.input, "@src/ ");
}

#[test]
fn empty_cache_results_show_empty_not_hidden() {
    let mut lx = Lx::new();
    lx.set_mention_root(Some(std::path::PathBuf::from("/root")));
    lx.insert_char('@');
    let (generation, _) = lx.take_mention_scan_request().expect("request");
    assert!(lx.apply_mention_entries(generation, Path::new("/root"), Vec::new()));
    assert!(!lx.mention_open(), "空结果无候选面板");
    assert_eq!(
        lx.mention().view(&lx.input, lx.cursor),
        LxMentionView::Empty,
        "空结果显示固定尺寸状态行"
    );
}

#[test]
fn no_trigger_hides_panel() {
    let lx = ready("plain text", entries());
    assert_eq!(
        lx.mention().view(&lx.input, lx.cursor),
        LxMentionView::Hidden
    );
}

#[test]
fn loading_view_before_scan_returns() {
    let mut lx = Lx::new();
    lx.set_mention_root(Some(std::path::PathBuf::from("/root")));
    lx.insert_char('@');
    assert_eq!(
        lx.mention().view(&lx.input, lx.cursor),
        LxMentionView::Loading,
        "尚未取得缓存时显示扫描中"
    );
}

#[test]
fn deletion_closes_panel_when_trigger_disappears() {
    let mut lx = ready("@src/", entries());
    assert!(lx.mention_open(), "目录范围候选可见");
    for _ in 0..5 {
        lx.backspace();
    }
    assert!(lx.input.is_empty());
    assert!(!lx.mention_open());
}

#[test]
fn root_change_to_unavailable_dir_does_not_fall_back_to_old_candidates() {
    // 根 A 候选已就绪。
    let mut lx = ready("@", entries());
    assert!(lx.mention_open());
    // `cd` 到新目录：根切换即作废旧候选；即使新一轮扫描为空/不可访问也不回退。
    assert!(lx.set_mention_root(Some(std::path::PathBuf::from("/gone"))));
    assert!(!lx.mention_open(), "根变化立即作废旧候选，不等新结果");
    assert_eq!(
        lx.mention().view(&lx.input, lx.cursor),
        LxMentionView::Loading,
        "旧根结果不沿用：进入等待新扫描"
    );
    // 新根扫描返回空：变为空状态，不显示旧候选。
    let (generation, root) = lx.take_mention_scan_request().expect("rescan");
    assert!(lx.apply_mention_entries(generation, &root, Vec::new()));
    assert!(!lx.mention_open());
    assert_eq!(
        lx.mention().view(&lx.input, lx.cursor),
        LxMentionView::Empty
    );
}

#[test]
fn refresh_after_edit_resets_highlight_to_first() {
    // 过滤词变化时对齐 codex 重置高亮至首项 (0)。
    let mut lx = ready("@src/", entries());
    let panel = lx.mention().panel().expect("panel");
    assert!(panel.items().len() >= 2);
    lx.mention_set_active(1);
    assert_eq!(lx.mention().panel().expect("panel").active(), 1);
    lx.insert_char('l');
    let panel = lx.mention().panel().expect("panel after filter");
    assert_eq!(panel.active(), 0, "模糊匹配后高亮必须重置为首项");
}

#[test]
fn backspace_after_mention_space_deletes_whole_mention() {
    let mut lx = ready("@main", entries());
    assert!(lx.mention_confirm());
    assert_eq!(lx.input, "@main.rs ");
    // 光标紧跟尾随空格：一次退格删净整块提及。
    lx.backspace();
    assert_eq!(lx.input, "", "整块删除 @提及 ");
    lx.undo();
    assert_eq!(lx.input, "@main.rs ", "整块删除是一步撤销");
}

#[test]
fn backspace_without_trailing_space_deletes_one_char() {
    let mut lx = ready("@main", entries());
    assert!(lx.mention_confirm());
    // 光标移到尾随空格之前：退格只删一个字符。
    lx.move_left();
    lx.backspace();
    assert_eq!(lx.input, "@main.r ", "非空白处普通退格");
}

#[test]
fn backspace_while_panel_open_edits_query() {
    // 面板打开（@查询未闭合）时退格按普通编辑删除查询字符，不整块删除。
    let mut lx = ready("@main", entries());
    assert!(lx.mention_open());
    lx.backspace();
    assert_eq!(lx.input, "@mai", "面板打开时退格编辑查询");
}

fn dummy_ref_folders() -> Vec<crate::app::referenced::ReferencedFolder> {
    vec![
        crate::app::referenced::ReferencedFolder {
            id: 1,
            path: std::path::PathBuf::from("/Users/yonah/projects/herdr-master"),
        },
        crate::app::referenced::ReferencedFolder {
            id: 2,
            path: std::path::PathBuf::from("/Users/yonah/projects/lx-agent"),
        },
    ]
}

#[test]
fn mention_tab_switches_and_confirms_ref_dir() {
    let mut lx = ready("@agent", entries());
    lx.sync_referenced_folders(&dummy_ref_folders());
    // `@agent` 在 Files 下无匹配，但面板仍处于可见态（Empty），允许切到 refDir。
    assert_eq!(
        lx.mention().view(&lx.input, lx.cursor),
        LxMentionView::Empty
    );
    assert!(lx.mention_visible());
    assert_eq!(lx.mention_tab(), MentionTab::Files);

    // 切到 refDir
    assert!(lx.mention_switch_tab(1));
    assert_eq!(lx.mention_tab(), MentionTab::RefDir);
    assert!(lx.mention_open(), "refDir 下有匹配项，面板可交互");
    assert_eq!(lx.mention().filtered_ref_folders().len(), 1);
    assert!(
        !lx.mention_enter_folder(),
        "refDir 下 Shift+Enter 不进入目录"
    );

    // 确认插入 @[refer-folder](...)
    assert!(lx.mention_confirm());
    assert_eq!(lx.input, "@[refer-folder](/Users/yonah/projects/lx-agent) ");
    assert!(!lx.mention_visible(), "确认后关闭面板");
    assert_eq!(
        lx.mention_tab(),
        MentionTab::Files,
        "会话结束后复位回 Files"
    );
}

#[test]
fn mention_ref_dir_navigation_and_scroll_viewport() {
    let mut lx = ready("@", entries());
    lx.sync_referenced_folders(&dummy_ref_folders());
    assert!(lx.mention_switch_tab(1));
    assert_eq!(lx.mention_tab(), MentionTab::RefDir);
    assert_eq!(lx.mention().ref_active(), 0);

    assert!(lx.mention_move(1));
    assert_eq!(lx.mention().ref_active(), 1);
    assert!(lx.mention_move(1));
    assert_eq!(lx.mention().ref_active(), 0, "循环回到首项");

    assert!(lx.mention_scroll(1, 0));
    assert_eq!(lx.mention().ref_viewport(), Some(1));
    assert_eq!(lx.mention().ref_active(), 0, "滚轮不改高亮");

    // 修改查询词时重置 ref_viewport，且当结果缩减到 1 项时将高亮钳制到末项。
    lx.mention_set_active(1);
    for ch in "agent".chars() {
        lx.insert_char(ch);
    }
    assert_eq!(lx.mention().ref_viewport(), None, "查询词变化重置滚轮视口");
    assert_eq!(lx.mention().ref_active(), 0, "越界高亮钳制到末项");
}

#[test]
fn mention_ref_tab_typing_resets_selected_item_to_first() {
    let mut lx = ready("@", entries());
    lx.sync_referenced_folders(&dummy_ref_folders());
    assert!(lx.mention_switch_tab(1));
    assert_eq!(lx.mention_tab(), MentionTab::RefDir);
    assert!(lx.mention().filtered_ref_folders().len() >= 2);

    // 手动移动到第 1 项
    assert!(lx.mention_move(1));
    assert_eq!(lx.mention().ref_active(), 1);

    // 输入字符过滤，过滤词发生变动，即使结果仍有多项，高亮也必须重置为第 0 项
    lx.insert_char('p');
    assert_eq!(
        lx.mention().ref_active(),
        0,
        "过滤词变动后 secondary tab 高亮重置为首项"
    );

    // 再次手动下移
    if lx.mention().filtered_ref_folders().len() >= 2 {
        assert!(lx.mention_move(1));
        assert_eq!(lx.mention().ref_active(), 1);

        // 退格变动过滤词，高亮再次重置为首项
        lx.backspace();
        assert_eq!(lx.mention().ref_active(), 0, "退格后高亮重置为首项");
    }
}

#[test]
fn mention_files_tab_origin_change_resets_selected_item_to_first() {
    let mut lx = ready("@", entries());
    let panel = lx.mention().panel().expect("panel");
    assert!(panel.items().len() >= 2);
    lx.mention_set_active(1);
    assert_eq!(lx.mention().panel().expect("panel").active(), 1);

    // 追加文本输入新的 @ 触发点，虽然 query 同样是空串，高亮重置为首项 (0)
    lx.insert_char(' ');
    lx.insert_char('@');
    let panel = lx.mention().panel().expect("panel");
    assert_eq!(panel.active(), 0, "跨触发起点切换时高亮必须重置为首项");
}
