use super::*;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-skills-discover-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn skill(dir: &Path, name: &str) {
    let root = dir.join(name);
    std::fs::create_dir_all(&root).expect("skill dir");
    std::fs::write(
        root.join("SKILL.md"),
        format!("---\ndescription: {name} description\n---\nbody\n"),
    )
    .expect("skill file");
}

#[test]
fn project_roots_take_priority_and_duplicates_are_shadowed() {
    let base = temp_dir("priority");
    let user = base.join("user");
    let agents = base.join("agents");
    let project = base.join("project");
    let project_agents = base.join("project-agents");
    skill(&user, "shared");
    skill(&user, "user-only");
    skill(&project, "shared");
    skill(&project_agents, "project-only");
    let roots = ProjectSkillRoots {
        lx: Some(project),
        agents: Some(project_agents),
    };
    let catalog = discover(
        &user,
        &agents,
        Some(&roots),
        &HashSet::new(),
        &HashSet::new(),
    );
    let shared: Vec<_> = catalog
        .entries
        .iter()
        .filter(|entry| entry.name == "shared")
        .collect();
    assert_eq!(shared.len(), 2);
    assert!(shared[0].shadowed_by.is_none());
    assert!(shared[1].shadowed_by.is_some());
    assert_eq!(catalog.entries[0].scope, SkillScope::Project);
    assert_eq!(catalog.entries[0].source, SkillSource::Lx);
    assert!(
        catalog
            .entries
            .iter()
            .any(|entry| entry.name == "user-only")
    );
    assert!(
        catalog
            .entries
            .iter()
            .any(|entry| entry.name == "project-only")
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn hidden_directories_are_skipped_and_disabled_entries_are_off() {
    let base = temp_dir("hidden");
    let user = base.join("user");
    let agents = base.join("agents");
    skill(&user, "visible");
    skill(&user, ".hidden");
    let disabled_path = std::fs::canonicalize(user.join("visible/SKILL.md")).expect("canonical");
    let catalog = discover(
        &user,
        &agents,
        None,
        &HashSet::from([disabled_path]),
        &HashSet::new(),
    );
    assert!(catalog.entries.iter().all(|entry| entry.name != ".hidden"));
    let visible = catalog
        .entries
        .iter()
        .find(|entry| entry.name == "visible")
        .expect("visible");
    assert!(!visible.enabled);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn depth_limit_is_six() {
    let base = temp_dir("depth");
    let user = base.join("user");
    let agents = base.join("agents");
    let mut dir = user.clone();
    for index in 0..7 {
        dir = dir.join(format!("d{index}"));
    }
    std::fs::create_dir_all(&dir).expect("deep dir");
    std::fs::write(dir.join("SKILL.md"), "---\ndescription: too deep\n---\n").expect("file");
    let catalog = discover(&user, &agents, None, &HashSet::new(), &HashSet::new());
    assert!(
        catalog.entries.is_empty(),
        "第 8 层目录里的 skill 不得被发现"
    );
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn symlink_cycles_are_not_followed_forever() {
    let base = temp_dir("cycle");
    let user = base.join("user");
    let agents = base.join("agents");
    skill(&user, "real");
    std::os::unix::fs::symlink(&user, user.join("real").join("loop")).expect("symlink");
    let catalog = discover(&user, &agents, None, &HashSet::new(), &HashSet::new());
    assert_eq!(
        catalog
            .entries
            .iter()
            .filter(|entry| entry.name == "real")
            .count(),
        1
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn catalog_signature_tracks_added_and_edited_skills() {
    let base = temp_dir("signature");
    let user = base.join("user");
    let agents = base.join("agents");
    std::fs::create_dir_all(&user).expect("user");
    let first = catalog_signature(&user, &agents, None);
    skill(&user, "demo");
    let second = catalog_signature(&user, &agents, None);
    assert_ne!(first, second, "new SKILL.md changes the signature");
    let path = user.join("demo").join("SKILL.md");
    let mut text = std::fs::read_to_string(&path).expect("read");
    text.push_str("\nmore text\n");
    std::fs::write(&path, text).expect("write");
    let third = catalog_signature(&user, &agents, None);
    assert_ne!(second, third, "edited SKILL.md changes the signature");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn lowercase_skill_md_is_discovered() {
    let base = temp_dir("lowercase");
    let user = base.join("user");
    let agents = base.join("agents");
    let dir = user.join("grill-me");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(
        dir.join("skill.md"),
        "---\nname: grill-me\ndescription: Stress-test a plan.\n---\n\n# Grill\n",
    )
    .expect("skill");
    let catalog = discover(&user, &agents, None, &HashSet::new(), &HashSet::new());
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.name == "grill-me")
        .expect("lowercase skill discovered");
    assert!(
        entry.path.ends_with("grill-me/skill.md"),
        "{:?}",
        entry.path
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn agents_skills_are_disabled_until_explicitly_enabled() {
    let base = temp_dir("agents-default");
    let user = base.join("user");
    let agents = base.join("agents");
    skill(&agents, "agent-only");
    let path = std::fs::canonicalize(agents.join("agent-only/SKILL.md")).expect("canonical");
    let catalog = discover(&user, &agents, None, &HashSet::new(), &HashSet::new());
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.name == "agent-only")
        .expect("agent skill discovered");
    assert!(!entry.enabled, ".agents skill must default to disabled");
    let catalog = discover(
        &user,
        &agents,
        None,
        &HashSet::new(),
        &HashSet::from([path]),
    );
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.name == "agent-only")
        .expect("agent skill discovered");
    assert!(entry.enabled, "explicit enable must load the skill");
    let _ = std::fs::remove_dir_all(base);
}
