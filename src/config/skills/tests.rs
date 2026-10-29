use super::*;

#[test]
fn disabled_round_trip_and_permissions() {
    let dir = std::env::temp_dir().join(format!(
        "lx-skills-config-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    let path = dir.join("skills.json");
    let file = SkillsConfigFile {
        skills: SkillsSection {
            disabled: vec![PathBuf::from("/x/skills/pdf/SKILL.md")],
        },
    };
    save_skills(&path, &file).expect("save");
    let loaded = load_skills(&path).expect("load").expect("present");
    assert_eq!(loaded, file);
    assert_eq!(loaded.disabled_set().len(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn malformed_config_is_an_error_not_empty() {
    assert!(parse_skills("{\"skills\": 3}").is_err());
}

#[test]
fn project_roots_use_scope_directory() {
    let root = project_skills_root(Path::new("/repo/lx-tui")).expect("root");
    let text = root.to_string_lossy();
    assert!(text.contains("skills/projects/lx-tui-"));
    assert_eq!(
        project_agents_skills_root(Path::new("/repo")),
        PathBuf::from("/repo/.agents/skills")
    );
}
