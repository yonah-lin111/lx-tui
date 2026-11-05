//! `config::subagents` 单元测试：默认配置、覆写与自定义角色校验、角色解析与 0600 原子持久化。

use std::path::PathBuf;

use super::*;
use crate::config::ai::ModelSelection;

fn write_role_json(dir: &std::path::Path, file: &str, body: &str) {
    std::fs::create_dir_all(dir).expect("create role dir");
    std::fs::write(dir.join(file), body).expect("write role file");
}

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
    assert_eq!(section.max_depth, DEFAULT_MAX_AGENT_DEPTH);
    assert_eq!(section.default_max_tokens, None);
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
            max_concurrent: 12,
            max_depth: 3,
            default_max_tokens: Some(100_000),
            delegation_mode: DelegationMode::Auto,
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
                    ..RoleOverrideConfig::default()
                },
                executor: RoleOverrideConfig::default(),
                monitor: RoleOverrideConfig::default(),
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
                ..CustomRoleConfig::default()
            }],
        },
    };

    save_subagents(&path, &file).expect("save");
    let loaded = load_subagents(&path).expect("load").expect("file exists");
    assert_eq!(loaded.subagents.max_concurrent, 12);
    assert_eq!(loaded.subagents.max_depth, 3);
    assert_eq!(loaded.subagents.default_max_tokens, Some(100_000));
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
    assert_eq!(loaded.subagents.all_roles().len(), 4);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn monitor_base_role_roundtrips_through_serde_and_parse() {
    let monitor = BaseRole::Monitor;
    assert_eq!(monitor.as_str(), "monitor");
    assert_eq!(BaseRole::parse(" monitor "), Some(monitor));
    assert_eq!(
        serde_json::from_str::<BaseRole>("\"monitor\"").expect("deserialize monitor"),
        monitor
    );
    assert_eq!(
        serde_json::to_string(&monitor).expect("serialize monitor"),
        "\"monitor\""
    );
    assert!(monitor.default_description().contains("monitor"));

    let section = SubagentsSection {
        max_depth: 3,
        ..SubagentsSection::default()
    };
    let resolved = section.resolve_role("monitor").expect("monitor role");
    assert_eq!(resolved.base_role, BaseRole::Monitor);
    assert!(resolved.is_builtin);
    assert_eq!(resolved.system_prompt_override, None);
    assert_eq!(section.all_roles().len(), 3);
}

#[test]
fn delegation_mode_serializes_to_stable_strings() {
    assert_eq!(DelegationMode::default(), DelegationMode::Auto);
    for (mode, raw) in [
        (DelegationMode::Auto, "auto"),
        (DelegationMode::Proactive, "proactive"),
        (DelegationMode::Explicit, "explicit"),
    ] {
        assert_eq!(mode.as_str(), raw);
        assert_eq!(DelegationMode::parse(raw), Some(mode));
        assert_eq!(
            serde_json::to_string(&mode).expect("serialize mode"),
            format!("\"{raw}\"")
        );
        assert_eq!(
            serde_json::from_str::<DelegationMode>(&format!("\"{raw}\"")).expect("parse mode"),
            mode
        );
    }
    assert_eq!(DelegationMode::parse("always"), None);

    let parsed = parse_subagents(r#"{"subagents":{"delegationMode":"proactive"}}"#)
        .expect("parse delegation mode");
    assert_eq!(parsed.subagents.delegation_mode, DelegationMode::Proactive);
}

#[test]
fn role_capabilities_and_lock_model_serialize_flat() {
    let role = CustomRoleConfig {
        name: "reviewer".into(),
        base_role: BaseRole::Explorer,
        description: "Review code.".into(),
        system_prompt: "Review code.".into(),
        lock_model: true,
        capabilities: RoleCapabilities {
            allow_web_search: false,
            allow_mcp: false,
            allow_skills: true,
        },
        ..CustomRoleConfig::default()
    };
    let value = serde_json::to_value(&role).expect("serialize custom role");
    assert_eq!(value["lockModel"], true);
    assert_eq!(value["allowWebSearch"], false);
    assert_eq!(value["allowMcp"], false);
    assert_eq!(value["allowSkills"], true);
    let decoded: CustomRoleConfig = serde_json::from_value(value).expect("deserialize");
    assert!(decoded.lock_model);
    assert!(!decoded.capabilities.allow_web_search);
    assert!(!decoded.capabilities.allow_mcp);
    assert!(decoded.capabilities.allow_skills);

    // 缺省值：锁定关闭，三项能力全部开启。
    let defaults: RoleOverrideConfig =
        serde_json::from_str("{}").expect("deserialize empty override");
    assert!(!defaults.lock_model);
    assert_eq!(defaults.capabilities, RoleCapabilities::default());
    assert!(defaults.is_empty());

    let override_config = RoleOverrideConfig {
        lock_model: true,
        capabilities: RoleCapabilities {
            allow_web_search: false,
            allow_mcp: true,
            allow_skills: false,
        },
        ..RoleOverrideConfig::default()
    };
    let value = serde_json::to_value(&override_config).expect("serialize override");
    assert_eq!(value["lockModel"], true);
    assert_eq!(value["allowWebSearch"], false);
    assert_eq!(value["allowMcp"], true);
    assert_eq!(value["allowSkills"], false);
}

#[test]
fn validation_rejects_monitor_as_custom_role_name() {
    for name in ["explorer", "executor", "monitor"] {
        let section = SubagentsSection {
            roles: vec![CustomRoleConfig {
                name: name.into(),
                base_role: BaseRole::Executor,
                description: "desc".into(),
                system_prompt: "prompt".into(),
                ..CustomRoleConfig::default()
            }],
            ..SubagentsSection::default()
        };
        let error = section.validate().expect_err("reserved name rejected");
        assert!(error.to_string().contains("reserved"), "{name}: {error}");
    }
}

#[test]
fn discovers_project_roles_when_trusted_and_defaults_name_to_stem() {
    let workspace = temp_dir("project-discover");
    write_role_json(
        &workspace.join(".lx").join("agents"),
        "reviewer.json",
        r#"{"baseRole":"explorer","description":"Review changes.","systemPrompt":"You review."}"#,
    );
    write_role_json(
        &workspace.join(".agents").join("roles"),
        "watcher.json",
        r#"{"name":"watcher","baseRole":"monitor","description":"Watch builds.","systemPrompt":"You watch.","lockModel":true,"allowWebSearch":false}"#,
    );

    let discovered = discover_project_roles(&workspace);
    assert_eq!(discovered.len(), 2);
    let reviewer = discovered
        .iter()
        .find(|role| role.name == "reviewer")
        .expect("reviewer discovered from file stem");
    assert_eq!(reviewer.base_role, BaseRole::Explorer);
    let watcher = discovered
        .iter()
        .find(|role| role.name == "watcher")
        .expect("watcher discovered");
    assert_eq!(watcher.base_role, BaseRole::Monitor);
    assert!(watcher.lock_model);
    assert!(!watcher.capabilities.allow_web_search);
    assert!(discovered.iter().all(|role| role.is_project_scoped));

    let base = SubagentsSection::default();
    let merged = merge_project_roles(&base, Some(workspace.as_path()), true);
    assert_eq!(merged.roles.len(), 2);
    assert_eq!(base.roles.len(), 0, "base section is not mutated");

    let untrusted = merge_project_roles(&base, Some(workspace.as_path()), false);
    assert!(untrusted.roles.is_empty());
    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn project_roles_cannot_shadow_builtins_or_global_custom_roles() {
    let workspace = temp_dir("project-shadow");
    let roles_dir = workspace.join(".lx").join("agents");
    for name in ["explorer", "executor", "monitor"] {
        write_role_json(
            &roles_dir,
            &format!("{name}.json"),
            &format!(r#"{{"name":"{name}","description":"Hijack.","systemPrompt":"Hijack."}}"#),
        );
    }
    write_role_json(
        &roles_dir,
        "reviewer.json",
        r#"{"description":"Shadowed global role.","systemPrompt":"Shadowed."}"#,
    );
    write_role_json(
        &roles_dir,
        "auditor.json",
        r#"{"description":"Independent auditor.","systemPrompt":"You audit."}"#,
    );

    let base = SubagentsSection {
        roles: vec![CustomRoleConfig {
            name: "reviewer".into(),
            base_role: BaseRole::Explorer,
            description: "Global reviewer.".into(),
            system_prompt: "You review.".into(),
            ..CustomRoleConfig::default()
        }],
        ..SubagentsSection::default()
    };
    let discovered = discover_project_roles(&workspace);
    assert_eq!(
        discovered
            .iter()
            .map(|role| role.name.as_str())
            .collect::<Vec<_>>(),
        vec!["auditor", "reviewer"],
        "built-in names are skipped before merging"
    );

    let merged = merge_project_roles(&base, Some(workspace.as_path()), true);
    assert_eq!(merged.roles.len(), 2);
    assert_eq!(merged.roles[0].name, "reviewer");
    assert_eq!(merged.roles[0].description, "Global reviewer.");
    assert_eq!(merged.roles[1].name, "auditor");
    assert!(merged.roles[1].is_project_scoped);
    let _ = std::fs::remove_dir_all(workspace);
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
            ..CustomRoleConfig::default()
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
            ..CustomRoleConfig::default()
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

#[test]
fn validate_accepts_depth_and_budget_boundaries() {
    for max_depth in MIN_AGENT_DEPTH..=MAX_AGENT_DEPTH_LIMIT {
        let section = SubagentsSection {
            max_depth,
            ..SubagentsSection::default()
        };
        assert!(section.validate().is_ok(), "max_depth {max_depth}");
    }
    for max_concurrent in MIN_CONCURRENT_SUBAGENTS..=MAX_CONCURRENT_SUBAGENTS_LIMIT {
        let section = SubagentsSection {
            max_concurrent,
            ..SubagentsSection::default()
        };
        assert!(
            section.validate().is_ok(),
            "max_concurrent {max_concurrent}"
        );
    }
    let min_budget = SubagentsSection {
        default_max_tokens: Some(MIN_SUBAGENT_MAX_TOKENS),
        ..SubagentsSection::default()
    };
    assert!(min_budget.validate().is_ok());
    let unlimited = SubagentsSection {
        default_max_tokens: None,
        ..SubagentsSection::default()
    };
    assert!(unlimited.validate().is_ok());
}

#[test]
fn validate_rejects_out_of_range_depth_concurrency_and_budget() {
    for max_depth in [0, MAX_AGENT_DEPTH_LIMIT + 1] {
        let section = SubagentsSection {
            max_depth,
            ..SubagentsSection::default()
        };
        let error = section.validate().expect_err("invalid max_depth");
        assert!(
            error
                .to_string()
                .contains("maxDepth must be between 1 and 3"),
            "{error}"
        );
    }
    for max_concurrent in [0, MAX_CONCURRENT_SUBAGENTS_LIMIT + 1] {
        let section = SubagentsSection {
            max_concurrent,
            ..SubagentsSection::default()
        };
        let error = section.validate().expect_err("invalid max_concurrent");
        assert!(
            error
                .to_string()
                .contains("maxConcurrent must be between 1 and 12"),
            "{error}"
        );
    }
    let tiny_budget = SubagentsSection {
        default_max_tokens: Some(MIN_SUBAGENT_MAX_TOKENS - 1),
        ..SubagentsSection::default()
    };
    let error = tiny_budget
        .validate()
        .expect_err("invalid default_max_tokens");
    assert!(
        error
            .to_string()
            .contains("defaultMaxTokens must be at least 1000"),
        "{error}"
    );
}

#[test]
fn old_json_without_new_fields_uses_defaults_and_omits_none_budget() {
    let parsed = parse_subagents(r#"{"subagents":{}}"#).expect("parse legacy config");
    assert_eq!(parsed.subagents.max_concurrent, 4);
    assert_eq!(parsed.subagents.max_depth, 2);
    assert_eq!(parsed.subagents.default_max_tokens, None);

    let section = SubagentsSection::default();
    let serialized = serde_json::to_value(&section).expect("serialize");
    assert_eq!(serialized["maxDepth"], 2);
    assert!(serialized.get("defaultMaxTokens").is_none());

    let with_budget = SubagentsSection {
        default_max_tokens: Some(50_000),
        ..SubagentsSection::default()
    };
    let serialized = serde_json::to_value(&with_budget).expect("serialize");
    assert_eq!(serialized["defaultMaxTokens"], 50_000);
}
