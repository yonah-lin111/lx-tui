//! `lx-tui command` 子命令执行与文件读写。

use super::*;

pub fn run(
    cli: CommandCli,
    linked_worktree: Option<&Path>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let ctx = WorkspaceContext::resolve(cli.global.cwd.as_deref(), linked_worktree)?;
    let out = Output::new(cli.global.json, cli.global.color);
    let trusted = is_trusted(&ctx)?;
    match cli.command {
        CommandSubcommand::List { scope, kind } => run_list(&ctx, &out, trusted, scope, kind)?,
        CommandSubcommand::Show { name, scope, kind } => {
            run_show(&ctx, &out, trusted, &name, scope, kind)?
        }
        CommandSubcommand::Create(args) => run_create(&ctx, &out, trusted, &args)?,
        CommandSubcommand::Edit(args) => run_edit(&ctx, &out, trusted, &args)?,
        CommandSubcommand::Rename {
            old_name,
            new_name,
            scope,
            kind,
        } => run_rename(&ctx, &out, trusted, &old_name, &new_name, scope, kind)?,
        CommandSubcommand::Delete { name, scope, kind } => {
            run_delete(&ctx, &out, trusted, &name, scope, kind)?
        }
    }
    Ok(EXIT_OK)
}

fn is_trusted(ctx: &WorkspaceContext) -> Result<bool, String> {
    let db = ctx.open_db()?;
    Ok(ctx.is_trusted(&db))
}

fn ensure_single_line(field: &str, value: &str) -> Result<(), String> {
    if value.contains('\n') || value.contains('\r') {
        return Err(format!("{field} must not contain line breaks"));
    }
    Ok(())
}

fn user_root() -> Result<PathBuf, String> {
    crate::config::user_data_root().map_err(|error| error.to_string())
}

fn scope_root(scope: CommandScope, ctx: &WorkspaceContext) -> Result<PathBuf, String> {
    match scope {
        CommandScope::User => user_root(),
        CommandScope::Project => Ok(ctx.workspace.clone()),
    }
}

fn require_trusted(ctx: &WorkspaceContext, trusted: bool) -> Result<(), String> {
    if trusted {
        return Ok(());
    }
    Err(format!(
        "workspace '{}' is not trusted; run 'lx-tui workspace trust' first",
        ctx.workspace.display()
    ))
}

fn to_command_scope(scope: ScopeArg) -> CommandScope {
    match scope {
        ScopeArg::User => CommandScope::User,
        ScopeArg::Project => CommandScope::Project,
    }
}

fn find_record(
    ctx: &WorkspaceContext,
    trusted: bool,
    name: &str,
    kind: Option<CommandKind>,
    scope: Option<ScopeArg>,
) -> Result<(CommandRecord, CommandScope), String> {
    let user = commands::list(CommandScope::User, &user_root()?).0;
    let project = if trusted {
        commands::list(CommandScope::Project, &ctx.workspace).0
    } else {
        Vec::new()
    };
    let matches = |record: &CommandRecord| {
        record.name == name && kind.is_none_or(|expected| record.kind == expected)
    };
    match scope {
        Some(ScopeArg::User) => user
            .into_iter()
            .find(|record| matches(record))
            .map(|record| (record, CommandScope::User))
            .ok_or_else(|| format!("command '{name}' was not found in the user scope")),
        Some(ScopeArg::Project) => {
            require_trusted(ctx, trusted)?;
            project
                .into_iter()
                .find(|record| matches(record))
                .map(|record| (record, CommandScope::Project))
                .ok_or_else(|| format!("command '{name}' was not found in the project scope"))
        }
        None => project
            .into_iter()
            .find(|record| matches(record))
            .map(|record| (record, CommandScope::Project))
            .or_else(|| {
                user.into_iter()
                    .find(|record| matches(record))
                    .map(|record| (record, CommandScope::User))
            })
            .ok_or_else(|| format!("command '{name}' was not found")),
    }
}

fn truncate(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{head}...")
    } else {
        head
    }
}

fn run_list(
    ctx: &WorkspaceContext,
    out: &Output,
    trusted: bool,
    scope: ListScopeArg,
    kind: Option<CommandKind>,
) -> Result<(), String> {
    let mut user = commands::list(CommandScope::User, &user_root()?).0;
    let mut project = if trusted {
        commands::list(CommandScope::Project, &ctx.workspace).0
    } else {
        Vec::new()
    };
    // 先按 kind 过滤再合并：项目同名异类型命令不应吞掉用户级目标类型。
    if let Some(kind) = kind {
        user.retain(|record| record.kind == kind);
        project.retain(|record| record.kind == kind);
    }
    let mut overridden: HashSet<String> = HashSet::new();
    let records: Vec<(CommandRecord, CommandScope)> = match scope {
        ListScopeArg::User => user
            .into_iter()
            .map(|record| (record, CommandScope::User))
            .collect(),
        ListScopeArg::Project => project
            .into_iter()
            .map(|record| (record, CommandScope::Project))
            .collect(),
        ListScopeArg::All => {
            let project_names: HashSet<String> =
                project.iter().map(|record| record.name.clone()).collect();
            let (merged, shadowed) = commands::merge(user, project);
            overridden = shadowed.into_iter().map(|record| record.name).collect();
            merged
                .into_iter()
                .map(|record| {
                    let record_scope = if project_names.contains(&record.name) {
                        CommandScope::Project
                    } else {
                        CommandScope::User
                    };
                    (record, record_scope)
                })
                .collect()
        }
    };
    let (mut rows, mut values) = (Vec::new(), Vec::new());
    for (record, record_scope) in &records {
        rows.push(vec![
            record.name.clone(),
            record.kind.as_str().into(),
            record_scope.as_str().into(),
            truncate(&record.description, 60),
            if overridden.contains(&record.name) {
                "yes"
            } else {
                "-"
            }
            .into(),
        ]);
        values.push(json!({"name": record.name, "kind": record.kind.as_str(), "scope": record_scope.as_str(), "description": record.description, "mdScope": record.scope.as_str(), "title": record.title, "blockType": record.block.as_str(), "path": record.path, "overridden": overridden.contains(&record.name)}));
    }
    out.emit(
        &json!({"trusted": trusted, "commands": values}),
        |out, w| {
            if rows.is_empty() {
                return writeln!(w, "No custom commands found.");
            }
            out.print_table(
                w,
                &["NAME", "KIND", "SCOPE", "DESCRIPTION", "OVERRIDES"],
                &rows,
            )
        },
    )
}

fn run_show(
    ctx: &WorkspaceContext,
    out: &Output,
    trusted: bool,
    name: &str,
    scope: Option<ScopeArg>,
    kind: Option<CommandKind>,
) -> Result<(), String> {
    let (record, record_scope) = find_record(ctx, trusted, name, kind, scope)?;
    let value = json!({"name": record.name, "kind": record.kind.as_str(), "scope": record_scope.as_str(), "description": record.description, "mdScope": record.scope.as_str(), "blockType": record.block.as_str(), "title": record.title, "argumentHint": record.argument_hint, "path": record.path, "body": record.body});
    out.emit(&value, |_, w| {
        writeln!(w, "{:<14}{}", "Name:", record.name)?;
        writeln!(w, "{:<14}{}", "Kind:", record.kind.as_str())?;
        writeln!(w, "{:<14}{}", "Scope:", record_scope.as_str())?;
        writeln!(w, "{:<14}{}", "Description:", record.description)?;
        writeln!(w, "{:<14}{}", "Path:", record.path.display())?;
        writeln!(w, "\n{}", record.body)
    })
}

fn command_path(scope: CommandScope, root: &Path, kind: CommandKind, name: &str) -> PathBuf {
    commands::commands_dir(scope, root, kind).join(format!("{name}.md"))
}

fn run_create(
    ctx: &WorkspaceContext,
    out: &Output,
    trusted: bool,
    args: &CreateArgs,
) -> Result<(), String> {
    if args.scope == ScopeArg::Project {
        require_trusted(ctx, trusted)?;
    }
    if args.kind == CommandKind::AgentMd && (args.block.is_some() || args.title.is_some()) {
        return Err("'--block' and '--title' apply only to agentBlock commands".into());
    }
    if args.kind == CommandKind::AgentBlock && args.md_scope.is_some() {
        return Err("'--md-scope' applies only to agentMD commands".into());
    }
    let scope = to_command_scope(args.scope);
    let root = scope_root(scope, ctx)?;
    ensure_single_line("description", &args.description)?;
    if let Some(title) = &args.title {
        ensure_single_line("title", title)?;
    }
    let (md_scope, block) = match args.kind {
        CommandKind::AgentMd => (
            args.md_scope.unwrap_or(MdScope::Global),
            BlockType::Template,
        ),
        CommandKind::AgentBlock => (MdScope::Template, args.block.unwrap_or(BlockType::Template)),
    };
    let body = args.content.resolve(false)?.unwrap_or_default();
    if body.trim().is_empty() {
        return Err("command content cannot be empty".into());
    }
    let record = CommandRecord {
        name: args.name.clone(),
        scope: md_scope,
        description: args.description.clone(),
        argument_hint: args.arg_hint.clone(),
        title: args.title.clone(),
        body,
        kind: args.kind,
        block,
        path: command_path(scope, &root, args.kind, &args.name),
    };
    commands::save(scope, &root, &record, args.overwrite).map_err(|error| error.to_string())?;
    out.emit(&json!({"name": record.name, "kind": record.kind.as_str(), "scope": scope.as_str(), "path": record.path}), |_, w| writeln!(w, "Command '{}' saved in the {} scope at {}.", record.name, scope.as_str(), record.path.display()))
}

fn run_edit(
    ctx: &WorkspaceContext,
    out: &Output,
    trusted: bool,
    args: &EditArgs,
) -> Result<(), String> {
    let (mut record, scope) = find_record(
        ctx,
        trusted,
        &args.name,
        args.kind,
        args.scope.or(Some(ScopeArg::User)),
    )?;
    if record.kind == CommandKind::AgentMd && (args.block.is_some() || args.title.is_some()) {
        return Err("'--block' and '--title' apply only to agentBlock commands".into());
    }
    if record.kind == CommandKind::AgentBlock && args.md_scope.is_some() {
        return Err("'--md-scope' applies only to agentMD commands".into());
    }
    if let Some(description) = &args.description {
        ensure_single_line("description", description)?;
        record.description = description.clone();
    }
    if let Some(hint) = &args.arg_hint {
        ensure_single_line("argument hint", hint)?;
        record.argument_hint = Some(hint.clone());
    }
    if let Some(title) = &args.title {
        ensure_single_line("title", title)?;
        record.title = Some(title.clone());
    }
    match (record.kind, args.md_scope, args.block) {
        (CommandKind::AgentMd, Some(md_scope), _) => record.scope = md_scope,
        (CommandKind::AgentBlock, _, Some(block)) => record.block = block,
        _ => {}
    }
    if let Some(body) = args.content.resolve(false)? {
        record.body = body;
    }
    if record.body.trim().is_empty() {
        return Err("command content cannot be empty".into());
    }
    commands::save(scope, &scope_root(scope, ctx)?, &record, true)
        .map_err(|error| error.to_string())?;
    out.emit(&json!({"name": record.name, "kind": record.kind.as_str(), "scope": scope.as_str(), "path": record.path}), |_, w| writeln!(w, "Command '{}' updated in the {} scope.", record.name, scope.as_str()))
}

fn run_rename(
    ctx: &WorkspaceContext,
    out: &Output,
    trusted: bool,
    old_name: &str,
    new_name: &str,
    scope: Option<ScopeArg>,
    kind: Option<CommandKind>,
) -> Result<(), String> {
    let (record, record_scope) =
        find_record(ctx, trusted, old_name, kind, scope.or(Some(ScopeArg::User)))?;
    let path = commands::rename(&record.path, new_name).map_err(|error| error.to_string())?;
    out.emit(&json!({"oldName": old_name, "newName": new_name, "scope": record_scope.as_str(), "path": path}), |_, w| writeln!(w, "Command '{old_name}' renamed to '{new_name}' in the {} scope.", record_scope.as_str()))
}

fn run_delete(
    ctx: &WorkspaceContext,
    out: &Output,
    trusted: bool,
    name: &str,
    scope: Option<ScopeArg>,
    kind: Option<CommandKind>,
) -> Result<(), String> {
    let (record, record_scope) =
        find_record(ctx, trusted, name, kind, scope.or(Some(ScopeArg::User)))?;
    commands::delete(&record.path).map_err(|error| error.to_string())?;
    out.emit(
        &json!({"name": name, "scope": record_scope.as_str()}),
        |_, w| {
            writeln!(
                w,
                "Command '{name}' deleted from the {} scope.",
                record_scope.as_str()
            )
        },
    )
}
