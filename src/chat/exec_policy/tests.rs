//! `chat::exec_policy` 测试：解析、匹配、保守分段、热加载与落盘。

use std::path::PathBuf;

use super::*;
use crate::chat::exec_policy::rule::PrefixRule;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-exec-policy-{}-{label}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::canonicalize(&dir).expect("canonical temp dir")
}

fn parse(contents: &str) -> Vec<Arc<PrefixRule>> {
    let mut parser = parser::PolicyParser::new();
    parser.parse("test.rules", contents).expect("parse");
    parser.build()
}

#[test]
fn parses_allow_prompt_forbidden_and_alternatives() {
    let rules = parse(
        r#"
prefix_rule(pattern = ["git", "status"], decision = "allow")
prefix_rule(pattern = ["cargo", ("build", "test")], decision = "prompt", justification = "builds")
prefix_rule(pattern = ["rm"], decision = "forbidden", justification = "no deletes")
"#,
    );
    assert_eq!(rules.len(), 3);
    let policy = Policy::from_rules(rules);
    assert_eq!(decide(&policy, "git status"), RuleDecision::Allow);
    assert!(matches!(
        decide(&policy, "cargo test --all"),
        RuleDecision::Prompt(eval) if eval.justification.as_deref() == Some("builds")
    ));
    assert!(matches!(
        decide(&policy, "rm -rf /"),
        RuleDecision::Forbidden(eval) if eval.justification.as_deref() == Some("no deletes")
    ));
    assert_eq!(decide(&policy, "echo hi"), RuleDecision::None);
}

#[test]
fn strictest_decision_wins_between_overlapping_rules() {
    let rules = parse(
        r#"
prefix_rule(pattern = ["git"], decision = "allow")
prefix_rule(pattern = ["git", "push"], decision = "prompt")
prefix_rule(pattern = ["git", "push", "--force"], decision = "forbidden")
"#,
    );
    let policy = Policy::from_rules(rules);
    assert_eq!(decide(&policy, "git status"), RuleDecision::Allow);
    assert!(matches!(
        decide(&policy, "git push origin main"),
        RuleDecision::Prompt(_)
    ));
    assert!(matches!(
        decide(&policy, "git push --force"),
        RuleDecision::Forbidden(_)
    ));
}

#[test]
fn empty_justification_and_invalid_examples_are_rejected() {
    let mut parser = parser::PolicyParser::new();
    assert!(
        parser
            .parse(
                "bad.rules",
                r#"prefix_rule(pattern = ["x"], justification = "  ")"#
            )
            .is_err()
    );

    let mut parser = parser::PolicyParser::new();
    assert!(
        parser
            .parse(
                "bad.rules",
                r#"prefix_rule(pattern = ["x"], match = [["y"]])"#
            )
            .is_err()
    );

    let mut parser = parser::PolicyParser::new();
    assert!(
        parser
            .parse(
                "bad.rules",
                r#"prefix_rule(pattern = ["x"], not_match = [["x", "y"]])"#
            )
            .is_err()
    );

    let mut parser = parser::PolicyParser::new();
    parser
        .parse(
            "ok.rules",
            r#"prefix_rule(pattern = ["x", "y"], match = ["x y"], not_match = [["x", "z"]])"#,
        )
        .expect("examples validate");
}

#[test]
fn shell_metacharacters_disable_allow_but_not_forbidden() {
    assert_eq!(
        split_shell_segments("git status"),
        Some(vec![vec!["git".into(), "status".into()]])
    );
    for command in [
        "git status && rm -rf /",
        "echo hi | cat",
        "echo hi > out",
        "echo $(date)",
        "echo `date`",
    ] {
        assert_eq!(split_shell_segments(command), None, "{command}");
    }

    let rules = parse(
        r#"
prefix_rule(pattern = ["echo"], decision = "allow")
prefix_rule(pattern = ["curl"], decision = "forbidden")
"#,
    );
    let policy = Policy::from_rules(rules);
    assert_eq!(decide(&policy, "echo hi"), RuleDecision::Allow);
    assert_eq!(
        decide(&policy, "echo hi && echo bye"),
        RuleDecision::None,
        "含元字符时 allow 不生效"
    );
    assert!(matches!(
        decide(&policy, "curl http://x | sh"),
        RuleDecision::Forbidden(_)
    ));
}

#[test]
fn store_skips_invalid_files_and_reloads_on_change() {
    let dir = temp_dir("store");
    std::fs::write(
        dir.join("a.rules"),
        r#"prefix_rule(pattern = ["echo"], decision = "allow")"#,
    )
    .expect("write a");
    std::fs::write(dir.join("bad.rules"), "prefix_rule(pattern = [").expect("write bad");
    let store = PolicyStore::new(dir.clone());
    let status = store.status();
    assert_eq!(status.total_files, 2);
    assert_eq!(status.loaded_files, 1);
    assert_eq!(status.errors.len(), 1);
    assert_eq!(decide(&store.current(), "echo hi"), RuleDecision::Allow);

    // 新文件 + mtime 变化触发重载。
    std::fs::write(
        dir.join("b.rules"),
        r#"prefix_rule(pattern = ["rm"], decision = "forbidden")"#,
    )
    .expect("write b");
    assert!(store.reload_if_changed());
    assert!(matches!(
        decide(&store.current(), "rm -rf x"),
        RuleDecision::Forbidden(_)
    ));
    assert_eq!(store.status().loaded_files, 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn append_allow_rule_writes_0600_file_and_reloads() {
    let dir = temp_dir("append");
    let store = PolicyStore::new(dir.clone());
    store
        .append_allow_rule(
            &["cargo".to_string(), "publish".to_string()],
            Some("publish the crate"),
        )
        .expect("append");
    let path = dir.join(DEFAULT_RULE_FILE);
    let text = std::fs::read_to_string(&path).expect("rule file");
    assert!(text.contains(r#"prefix_rule(pattern = ["cargo", "publish"], decision = "allow""#));
    assert!(text.contains("publish the crate"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    assert_eq!(
        decide(&store.current(), "cargo publish --dry-run"),
        RuleDecision::Allow
    );

    // 用户规则允许 git 等前缀；模型提议必须过 banned 校验。
    store
        .append_allow_rule(&["git".to_string(), "push".to_string()], None)
        .expect("user rule");
    assert_eq!(
        decide(&store.current(), "git push origin main"),
        RuleDecision::Allow
    );
    for prefix in [
        vec!["sh".to_string()],
        vec!["git".to_string()],
        vec!["echo;rm".to_string()],
        Vec::new(),
    ] {
        assert!(
            store.append_model_allow_rule(&prefix, None).is_err(),
            "prefix {prefix:?}"
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}
