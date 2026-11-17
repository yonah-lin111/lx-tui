//! `platform::sandbox::glob` 测试：SBPL 正则子集、扫描根与有界匹配。

use super::*;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-glob-{}-{label}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::canonicalize(&dir).expect("canonical temp dir")
}

#[test]
fn regex_covers_star_double_star_question_and_braces() {
    assert_eq!(
        to_regex("/a/*.log", false).as_deref(),
        Some("^/a/[^/]*\\.log$")
    );
    assert_eq!(
        to_regex("/a/**/*.log", false).as_deref(),
        Some("^/a/(.*/)?[^/]*\\.log$")
    );
    assert_eq!(
        to_regex("/a/?.txt", false).as_deref(),
        Some("^/a/[^/]\\.txt$")
    );
    assert_eq!(
        to_regex("/a/{b,c}.txt", false).as_deref(),
        Some("^/a/(b|c)\\.txt$")
    );
    assert_eq!(to_regex("/a/b", true).as_deref(), Some("^/a/b(/.*)?$"));
    assert_eq!(to_regex("", false), None);
}

#[test]
fn sbpl_literal_escapes_quotes() {
    assert_eq!(sbpl_regex_literal(r#"^/a"b$"#), r#"^/a\"b$"#);
}

#[test]
fn scan_root_uses_static_prefix_before_first_meta() {
    assert_eq!(scan_root("/ws/**/*.log").as_deref(), Some(Path::new("/ws")));
    assert_eq!(scan_root("/ws/a*/x").as_deref(), Some(Path::new("/ws")));
    assert_eq!(scan_root("/ws/plain").as_deref(), Some(Path::new("/ws")));
    assert_eq!(scan_root("/**/*.log"), None);
    assert_eq!(scan_root("/{a,b}/x"), None);
}

#[test]
fn scan_matches_is_depth_bounded_and_includes_matches() {
    let root = temp_dir("scan");
    std::fs::create_dir_all(root.join("a/b/c")).expect("nested");
    std::fs::write(root.join("top.log"), "x").expect("top");
    std::fs::write(root.join("a/mid.log"), "x").expect("mid");
    std::fs::write(root.join("a/b/c/deep.log"), "x").expect("deep");
    let pattern = format!("{}/**/*.log", root.display());
    let matches = scan_matches(&[pattern], 2).expect("scan");
    assert!(matches.contains(&root.join("top.log")));
    assert!(matches.contains(&root.join("a/mid.log")));
    assert!(
        !matches.contains(&root.join("a/b/c/deep.log")),
        "depth 2 上限"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn root_anchored_glob_is_rejected_instead_of_scanning_everything() {
    assert!(scan_matches(&["/**/*.log".to_string()], 4).is_err());
}
