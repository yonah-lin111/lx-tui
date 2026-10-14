//! 引用文件夹模型单元测试：筛选排序、子序列匹配、窗口与快照键语义。

use super::*;

fn entry(relative: &str, is_directory: bool) -> ReferencedEntry {
    let name = relative.rsplit('/').next().unwrap_or(relative).to_string();
    ReferencedEntry {
        path: PathBuf::from("/root").join(relative),
        relative: relative.to_string(),
        name,
        is_directory,
    }
}

#[test]
fn filter_matches_name_before_path_and_skips_root() {
    let entries = vec![
        entry("src/main.rs", false),
        entry("main/mod.rs", false),
        entry("docs/readme.md", false),
        entry("src", true),
    ];
    let (items, overflow) = filter_entries(&entries, "main");
    assert!(!overflow);
    // 名称前缀（`main.rs`）优先于路径前缀（`main/mod.rs`），两者都在路径包含之前。
    assert_eq!(items[0].relative, "src/main.rs");
    assert_eq!(items[1].relative, "main/mod.rs");
    assert!(!items.iter().any(|entry| entry.relative == "docs/readme.md"));
}

#[test]
fn filter_empty_query_lists_all_sorted_by_path() {
    let entries = vec![entry("b.txt", false), entry("a.txt", false)];
    let (items, overflow) = filter_entries(&entries, "");
    assert!(!overflow);
    assert_eq!(
        items
            .iter()
            .map(|entry| entry.relative.as_str())
            .collect::<Vec<_>>(),
        vec!["a.txt", "b.txt"]
    );
}

#[test]
fn filter_is_case_insensitive_and_supports_subsequence() {
    let entries = vec![entry("src/AppState.rs", false)];
    let (items, _) = filter_entries(&entries, "appstate");
    assert_eq!(items.len(), 1, "大小写不敏感名称包含");
    let (items, _) = filter_entries(&entries, "aps");
    assert_eq!(items.len(), 1, "名称子序列命中");
}

#[test]
fn filter_caps_results_and_reports_truncation() {
    let entries: Vec<ReferencedEntry> = (0..MAX_RESULTS + 10)
        .map(|index| entry(&format!("file-{index:04}.txt"), false))
        .collect();
    let (items, overflow) = filter_entries(&entries, "file");
    assert_eq!(items.len(), MAX_RESULTS);
    assert!(overflow, "超过展示上限时报告截断");
}

#[test]
fn menu_window_stays_within_bounds() {
    assert_eq!(menu_window_start(0, 5, 10), 0);
    assert_eq!(menu_window_start(9, 5, 10), 5);
    assert_eq!(menu_window_start(3, 5, 10), 0);
    assert_eq!(menu_window_start(4, 3, 3), 0, "列表不超过窗口时不滚动");
}

#[test]
fn snapshot_reset_and_apply_track_key() {
    let mut snapshot = ReferencedFolders::default();
    snapshot.reset(Some(PathBuf::from("/repo")));
    assert!(!snapshot.loaded);
    assert!(snapshot.folders.is_empty());
    snapshot.apply(
        PathBuf::from("/repo"),
        vec![ReferencedFolder {
            id: 1,
            path: PathBuf::from("/tmp/x"),
        }],
        false,
    );
    assert!(snapshot.loaded);
    assert!(snapshot.matches(std::path::Path::new("/repo")));
    assert!(!snapshot.matches(std::path::Path::new("/other")));
    snapshot.reset(Some(PathBuf::from("/other")));
    assert!(!snapshot.loaded);
    assert!(snapshot.folders.is_empty(), "键变化丢弃旧快照");
}

#[test]
fn display_name_falls_back_to_full_path() {
    let folder = ReferencedFolder {
        id: 1,
        path: PathBuf::from("/tmp/project"),
    };
    assert_eq!(folder.display_name(), "project");
}

#[test]
fn request_currentness_compares_ids() {
    assert!(request_is_current(3, Some(3)));
    assert!(!request_is_current(3, Some(4)));
    assert!(!request_is_current(3, None));
}
