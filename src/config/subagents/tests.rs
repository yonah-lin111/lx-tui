//! `config::subagents` 单元测试：默认配置、覆写与自定义角色校验、角色解析与 0600 原子持久化。

use std::path::PathBuf;

use super::*;
use crate::config::ai::ModelSelection;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-subagents-cfg-{}-{label}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn missing_file_returns_none_and_default_section_resolves_builtins() {
    let dir = temp_dir("missing");
    let path = dir.join("subagents.json");
    assert_eq!(load_subagents(&path).expect("load"), None);

    let section = SubagentsSection::default();
    assert_eq!(section.max_concurrent, DEFAULT_MAX_CONCURRENT_SUBAGENTS);
    let explorer = section.resolve_role("explorer").expect("explorer");
    assert_eq!(explorer.base_role, BaseRole::Explorer);
    assert!(explorer.is_builtin);
    assert_eq!(explorer.system_prompt_override, None);

    let executor = section.resolve_role("executor").expect("executor");
    assert_eq!(executor.base_role, BaseRole::Executor);
    assert!(executor.is_builtin);
    assert_eq!(section.resolve_role("reviewer"), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn save_and_load_round_trip_with_builtin_overrides_and_custom_roles() {
    let dir = temp_dir("roundtrip");
    let path = dir.join("config").join("subagents.json");
    let file = SubagentsConfigFile {
        subagents: SubagentsSection {
            max_concurrent: 5,
            builtins: BuiltinRoleOverrides {
                explorer: RoleOverrideConfig {
                    description: Some(" Fast read-only scout ".into()),
                    system_prompt: None,
                    model: Some(ModelSelection {
                        provider: "openai".into(),
                        model: "gpt-4.1-mini".into(),
                        variant: Some("low".into()),
                    }),
                    nickname_candidates: vec!["Turing".into(), "turing".into(), "Curie".into()],
                },
                executor: RoleOverrideConfig::default(),
            },
            roles: vec![CustomRoleConfig {
                name: "reviewer".into(),
                base_role: BaseRole::Explorer,
                description: "Strict security and regression reviewer.".into(),
                system_prompt: "You are a code reviewer.".into(),
                model: Some(ModelSelection {
                    provider: "anthropic".into(),
                    model: "claude-sonnet-4-5".into(),
                    variant: Some("high".into()),
                }),
                nickname_candidates: vec!["Dijkstra".into(), "Hoare".into()],
            }],
        },
    };

    save_subagents(&path, &file).expect("save");
    let loaded = load_subagents(&path).expect("load").expect("file exists");
    assert_eq!(loaded.subagents.max_concurrent, 5);
    assert_eq!(
        loaded.subagents.builtins.explorer.description.as_deref(),
        Some("Fast read-only scout")
    );
    // 大小写重复的昵称已去重。
    assert_eq!(
        loaded.subagents.builtins.explorer.nickname_candidates,
        vec!["Turing".to_string(), "Curie".to_string()]
    );

    let reviewer = loaded
        .subagents
        .resolve_role("reviewer")
        .expect("reviewer resolved");
    assert_eq!(reviewer.base_role, BaseRole::Explorer);
    assert!(!reviewer.is_builtin);
    assert_eq!(
        reviewer.system_prompt_override.as_deref(),
        Some("You are a code reviewer.")
    );
    assert_eq!(loaded.subagents.all_roles().len(), 3);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn validation_rejects_invalid_concurrency_reserved_or_duplicate_names() {
    let bad_conc = SubagentsSection {
        max_concurrent: 0,
        ..SubagentsSection::default()
    };
    assert!(bad_conc.validate().is_err());

    let reserved = SubagentsSection {
        roles: vec![CustomRoleConfig {
            name: "explorer".into(),
            base_role: BaseRole::Explorer,
            description: "dup".into(),
            system_prompt: "prompt".into(),
            model: None,
            nickname_candidates: Vec::new(),
        }],
        ..SubagentsSection::default()
    };
    assert!(reserved.validate().is_err());

    let bad_syntax = SubagentsSection {
        roles: vec![CustomRoleConfig {
            name: "Bad-Name".into(),
            base_role: BaseRole::Executor,
            description: "desc".into(),
            system_prompt: "prompt".into(),
            model: None,
            nickname_candidates: Vec::new(),
        }],
        ..SubagentsSection::default()
    };
    assert!(bad_syntax.validate().is_err());

    let comma_nickname = SubagentsSection {
        builtins: BuiltinRoleOverrides {
            explorer: RoleOverrideConfig {
                nickname_candidates: vec!["Turing,Ada".into()],
                ..RoleOverrideConfig::default()
            },
            ..BuiltinRoleOverrides::default()
        },
        ..SubagentsSection::default()
    };
    assert!(comma_nickname.validate().is_err());

    let incomplete_model = SubagentsConfigFile {
        subagents: SubagentsSection {
            builtins: BuiltinRoleOverrides {
                explorer: RoleOverrideConfig {
                    model: Some(ModelSelection {
                        provider: "openai".into(),
                        model: String::new(),
                        variant: None,
                    }),
                    ..RoleOverrideConfig::default()
                },
                ..BuiltinRoleOverrides::default()
            },
            ..SubagentsSection::default()
        },
    };
    let dir = temp_dir("incomplete-model");
    assert!(save_subagents(&dir.join("subagents.json"), &incomplete_model).is_err());
    let _ = std::fs::remove_dir_all(dir);
}
