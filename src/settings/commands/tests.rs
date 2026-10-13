use super::*;

use std::time::{SystemTime, UNIX_EPOCH};

/// 隔离临时目录：按测试名、进程号与时间戳唯一。
fn temp_dir(name: &str) -> PathBuf {
    let unique = format!(
        "lx-tui-commands-tests-{}-{}-{}",
        name,
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    );
    let path = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

/// 构造一条命令记录。
fn record(name: &str, kind: CommandKind, scope: MdScope, body: &str) -> CommandRecord {
    CommandRecord {
        name: name.to_string(),
        scope,
        description: format!("{name} description"),
        argument_hint: None,
        title: None,
        body: body.to_string(),
        kind,
        block: BlockType::Template,
        path: PathBuf::new(),
    }
}

#[test]
fn name_validation_accepts_safe_and_rejects_reserved_or_illegal() {
    assert!(validate_name("my-command_1").is_ok());
    assert!(validate_name("A").is_ok());
    assert!(validate_name("").is_err());
    assert!(validate_name("1abc").is_err());
    assert!(validate_name("bad/name").is_err());
    assert!(validate_name("bad name").is_err());
    assert!(validate_name("../escape").is_err());
    assert!(
        validate_name("add").is_err(),
        "built-in short name reserved"
    );
    assert!(validate_name("refactorTemplate").is_err());
    assert!(validate_name("gitWorkTree").is_err());
    assert!(validate_name(&"a".repeat(65)).is_err());
}

#[test]
fn parse_reads_frontmatter_and_body() {
    let text = "---\ndescription: review the diff\nargument-hint: <path>\nscope: template\n---\n# Body\ncontent\n";
    let record = parse_file(
        "review",
        text,
        CommandKind::AgentMd,
        Path::new("/x/review.md"),
    )
    .expect("parses");
    assert_eq!(record.name, "review");
    assert_eq!(record.description, "review the diff");
    assert_eq!(record.argument_hint.as_deref(), Some("<path>"));
    assert_eq!(record.scope, MdScope::Template);
    assert_eq!(record.body, "# Body\ncontent");
    assert_eq!(record.kind, CommandKind::AgentMd);
}

#[test]
fn parse_defaults_scope_to_global_for_agent_md() {
    let text = "---\ndescription: plain\n---\nbody";
    let record = parse_file(
        "plain",
        text,
        CommandKind::AgentMd,
        Path::new("/x/plain.md"),
    )
    .expect("parses");
    assert_eq!(record.scope, MdScope::Global);
}

#[test]
fn parse_block_requires_template_scope_and_keeps_title() {
    let text = "---\nblock: template\ntitle: My Block\n---\n&&& x --start\nbody\n&&& x --end";
    let record =
        parse_file("blk", text, CommandKind::AgentBlock, Path::new("/x/blk.md")).expect("parses");
    assert_eq!(record.kind, CommandKind::AgentBlock);
    assert_eq!(record.scope, MdScope::Template);
    assert_eq!(record.title.as_deref(), Some("My Block"));

    let bad = "---\nblock: nope\n---\nbody";
    assert!(parse_file("blk", bad, CommandKind::AgentBlock, Path::new("/x/blk.md")).is_err());
    let bad_scope = "---\nscope: global\n---\nbody";
    assert!(
        parse_file(
            "blk",
            bad_scope,
            CommandKind::AgentBlock,
            Path::new("/x/blk.md")
        )
        .is_err(),
        "agentBlock cannot be global"
    );
}

#[test]
fn parse_rejects_malformed_frontmatter_and_empty_body() {
    let malformed = "---\ndescription review\n---\nbody";
    assert!(
        parse_file(
            "bad",
            malformed,
            CommandKind::AgentMd,
            Path::new("/x/bad.md")
        )
        .is_err(),
        "line without colon is rejected"
    );
    let invalid_scope = "---\nscope: nowhere\n---\nbody";
    assert!(
        parse_file(
            "bad",
            invalid_scope,
            CommandKind::AgentMd,
            Path::new("/x/bad.md")
        )
        .is_err()
    );
    let empty = "---\ndescription: x\n---\n\n  \n";
    assert!(
        parse_file("bad", empty, CommandKind::AgentMd, Path::new("/x/bad.md")).is_err(),
        "empty body is rejected"
    );
}

#[test]
fn parse_without_frontmatter_treats_whole_text_as_body() {
    let record = parse_file(
        "raw",
        "# Title\nbody\n",
        CommandKind::AgentMd,
        Path::new("/x/raw.md"),
    )
    .expect("parses");
    assert!(record.description.is_empty());
    assert_eq!(record.scope, MdScope::Global);
    assert_eq!(record.body, "# Title\nbody");
}

#[test]
fn serialize_round_trips_through_parse() {
    let mut record = record(
        "cycle",
        CommandKind::AgentMd,
        MdScope::Template,
        "# Body\nline",
    );
    record.argument_hint = Some("<x>".into());
    let text = serialize(&record);
    let parsed = parse_file(
        "cycle",
        &text,
        CommandKind::AgentMd,
        Path::new("/x/cycle.md"),
    )
    .expect("parses");
    assert_eq!(parsed.description, record.description);
    assert_eq!(parsed.argument_hint, record.argument_hint);
    assert_eq!(parsed.scope, record.scope);
    assert_eq!(parsed.body, record.body);
}

#[test]
fn save_list_delete_round_trip_in_scope_root() {
    let root = temp_dir("crud");
    let user_root = root.join("user");
    let project_root = root.join("project");
    std::fs::create_dir_all(&user_root).expect("user root");
    std::fs::create_dir_all(&project_root).expect("project root");

    let mut first = record("alpha", CommandKind::AgentMd, MdScope::Global, "alpha body");
    first.path = PathBuf::new();
    save(CommandScope::User, &user_root, &first, false).expect("save user");
    save(
        CommandScope::Project,
        &project_root,
        &record(
            "beta",
            CommandKind::AgentBlock,
            MdScope::Template,
            "&&& b --start\nx\n&&& b --end",
        ),
        false,
    )
    .expect("save project");

    let (user, errors) = list(CommandScope::User, &user_root);
    assert!(errors.is_empty());
    assert_eq!(user.len(), 1);
    assert_eq!(user[0].name, "alpha");
    let (project, errors) = list(CommandScope::Project, &project_root);
    assert!(errors.is_empty());
    assert_eq!(project.len(), 1);
    assert_eq!(project[0].name, "beta");
    assert_eq!(project[0].kind, CommandKind::AgentBlock);

    delete(&user[0].path).expect("delete");
    let (user, _) = list(CommandScope::User, &user_root);
    assert!(user.is_empty());
}

#[test]
fn save_refuses_overwrite_without_flag_and_allows_with_it() {
    let root = temp_dir("overwrite");
    let scope_root = root.join("user");
    save(
        CommandScope::User,
        &scope_root,
        &record("dup", CommandKind::AgentMd, MdScope::Global, "one"),
        false,
    )
    .expect("first save");
    let error = save(
        CommandScope::User,
        &scope_root,
        &record("dup", CommandKind::AgentMd, MdScope::Global, "two"),
        false,
    )
    .expect_err("second save refused");
    assert!(matches!(error, CommandsError::AlreadyExists(_)));
    save(
        CommandScope::User,
        &scope_root,
        &record("dup", CommandKind::AgentMd, MdScope::Global, "two"),
        true,
    )
    .expect("overwrite");
    let (records, _) = list(CommandScope::User, &scope_root);
    assert_eq!(records[0].body, "two");
}

#[test]
fn save_rejects_reserved_name() {
    let root = temp_dir("reserved");
    let error = save(
        CommandScope::User,
        &root,
        &record("add", CommandKind::AgentMd, MdScope::Global, "x"),
        true,
    )
    .expect_err("reserved name rejected");
    assert!(matches!(error, CommandsError::InvalidName(_)));
}

#[test]
fn rename_moves_file_and_refuses_collision() {
    let root = temp_dir("rename");
    let scope_root = root.join("user");
    save(
        CommandScope::User,
        &scope_root,
        &record("old", CommandKind::AgentMd, MdScope::Global, "body"),
        false,
    )
    .expect("save");
    save(
        CommandScope::User,
        &scope_root,
        &record("taken", CommandKind::AgentMd, MdScope::Global, "other"),
        false,
    )
    .expect("save other");
    let (records, _) = list(CommandScope::User, &scope_root);
    let old = records.iter().find(|item| item.name == "old").expect("old");
    let error = rename(&old.path, "taken").expect_err("collision refused");
    assert!(matches!(error, CommandsError::AlreadyExists(_)));
    let moved = rename(&old.path, "new").expect("rename");
    assert!(moved.ends_with("new.md"));
    let (records, _) = list(CommandScope::User, &scope_root);
    let mut names: Vec<&str> = records.iter().map(|item| item.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["new", "taken"]);
}

#[test]
fn malformed_files_are_reported_but_valid_ones_stay_available() {
    let root = temp_dir("malformed");
    let dir = commands_dir(CommandScope::User, &root, CommandKind::AgentMd);
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(dir.join("good.md"), "---\ndescription: ok\n---\nbody").expect("good");
    std::fs::write(
        dir.join("bad.md"),
        "---\ndescription without colon\n---\nbody",
    )
    .expect("bad");
    std::fs::write(dir.join("add.md"), "---\ndescription: reserved\n---\nbody").expect("reserved");
    std::fs::write(dir.join("notes.txt"), "ignored").expect("non-md ignored");
    let (records, errors) = list(CommandScope::User, &root);
    let names: Vec<&str> = records.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(records.len(), 1, "records: {names:?} errors: {errors:?}");
    assert_eq!(records[0].name, "good");
    assert_eq!(
        errors.len(),
        2,
        "malformed frontmatter and reserved name are reported: {errors:?}"
    );
}

#[test]
fn project_overrides_user_on_name_collision() {
    let user = vec![
        record("shared", CommandKind::AgentMd, MdScope::Global, "user body"),
        record(
            "only-user",
            CommandKind::AgentMd,
            MdScope::Global,
            "user only",
        ),
    ];
    let project = vec![record(
        "shared",
        CommandKind::AgentMd,
        MdScope::Template,
        "project body",
    )];
    let (merged, overridden) = merge(user, project);
    assert_eq!(merged.len(), 2);
    let shared = merged
        .iter()
        .find(|item| item.name == "shared")
        .expect("shared");
    assert_eq!(shared.body, "project body");
    assert_eq!(overridden.len(), 1);
    assert_eq!(overridden[0].body, "user body");
    assert!(merged.iter().any(|item| item.name == "only-user"));
}

#[test]
fn directories_are_distinct_per_scope_and_kind() {
    let user = commands_dir(CommandScope::User, Path::new("/data"), CommandKind::AgentMd);
    assert!(user.ends_with("command/agentMD"));
    let project = commands_dir(
        CommandScope::Project,
        Path::new("/repo"),
        CommandKind::AgentBlock,
    );
    assert!(project.ends_with(".lx-t/command/agentBlock"));
    assert_eq!(CommandKind::parse("agentMD"), Some(CommandKind::AgentMd));
    assert_eq!(
        CommandKind::parse("agentBlock"),
        Some(CommandKind::AgentBlock)
    );
    assert_eq!(CommandKind::parse("nope"), None);
}

#[test]
fn split_frontmatter_requires_leading_delimiter_line() {
    let (front, body) = split_frontmatter("---\na: b\n---\nbody");
    assert_eq!(front, "a: b\n");
    assert_eq!(body, "body");
    // 不以 `---` 开头：整体是正文。
    let (front, body) = split_frontmatter("body\n---\nmore");
    assert!(front.is_empty());
    assert_eq!(body, "body\n---\nmore");
    // 未闭合 frontmatter：整体是正文（避免误吞内容）。
    let (front, body) = split_frontmatter("---\na: b\nbody");
    assert!(front.is_empty());
    assert_eq!(body, "---\na: b\nbody");
}

#[test]
fn block_type_parses_serializes_and_rejects_unknown() {
    let supple =
        "---\nblock: supple\ndescription: d\n---\n+++ note --start\nbody\n+++ note --end\n";
    let record = parse_file(
        "note",
        supple,
        CommandKind::AgentBlock,
        Path::new("/x/note.md"),
    )
    .expect("supple parses");
    assert_eq!(record.block, BlockType::Supple);
    assert!(serialize(&record).contains("block: supple"));

    let legacy = parse_file(
        "blk",
        "---\ndescription: d\n---\nbody",
        CommandKind::AgentBlock,
        Path::new("/x/blk.md"),
    )
    .expect("legacy parses");
    assert_eq!(legacy.block, BlockType::Template, "旧文件默认 template");
    assert!(serialize(&legacy).contains("block: template"));

    assert!(
        parse_file(
            "bad",
            "---\nblock: bogus\n---\nbody",
            CommandKind::AgentBlock,
            Path::new("/x/bad.md"),
        )
        .is_err(),
        "未知 block 值报错"
    );
}

#[test]
fn supple_and_var_names_are_reserved() {
    assert!(validate_name("supplement").is_err());
    assert!(validate_name("var").is_err());
    assert!(validate_name("singleLine").is_err());
    assert!(validate_name("multiLine").is_err());
    assert!(validate_name("varTemplate").is_err());
}
