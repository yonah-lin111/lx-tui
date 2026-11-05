//! `lx-tui skill` 子命令执行与技能目录读写。

use super::*;

pub fn run(
    cli: SkillCli,
    linked_worktree: Option<&Path>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let ctx = WorkspaceContext::resolve(cli.global.cwd.as_deref(), linked_worktree)?;
    let out = Output::new(cli.global.json, cli.global.color);
    let trusted = is_trusted(&ctx)?;
    match cli.command {
        SkillCommand::List { scope } => run_list(&ctx, &out, trusted, scope)?,
        SkillCommand::Show { name, scope } => run_show(&ctx, &out, trusted, &name, scope)?,
        SkillCommand::Enable { target } => run_toggle(&ctx, &out, trusted, &target, true)?,
        SkillCommand::Disable { target } => run_toggle(&ctx, &out, trusted, &target, false)?,
        SkillCommand::Create {
            name,
            description,
            content,
            scope,
        } => run_create(&ctx, &out, trusted, &name, &description, &content, scope)?,
        SkillCommand::Edit {
            name,
            description,
            content,
            scope,
        } => run_edit(
            &ctx,
            &out,
            trusted,
            &name,
            description.as_deref(),
            &content,
            scope,
        )?,
        SkillCommand::Delete { name, scope } => run_delete(&ctx, &out, trusted, &name, scope)?,
    }
    Ok(EXIT_OK)
}

fn is_trusted(ctx: &WorkspaceContext) -> Result<bool, String> {
    ctx.open_db()?
        .is_trusted(&ctx.workspace)
        .map_err(|error| error.code().to_string())
}

fn catalog(ctx: &WorkspaceContext, trusted: bool) -> Result<SkillCatalog, String> {
    let config = skills_config_path().map_err(|error| error.to_string())?;
    let file = load_skills(&config)
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    let disabled: HashSet<PathBuf> = file.disabled_set().into_iter().collect();
    let enabled: HashSet<PathBuf> = file.enabled_set().into_iter().collect();
    let user = user_skills_root().map_err(|error| error.to_string())?;
    let user_agents = user_agents_skills_root().map_err(|error| error.to_string())?;
    let project = if trusted {
        Some(ProjectSkillRoots::for_project(
            &ctx.workspace,
            project_skills_root(&ctx.workspace).map_err(|error| error.to_string())?,
        ))
    } else {
        None
    };
    Ok(crate::skills::discover::discover(
        &user,
        &user_agents,
        project.as_ref(),
        &disabled,
        &enabled,
    ))
}

fn scope_root(scope: SkillScope, ctx: &WorkspaceContext) -> Result<PathBuf, String> {
    match scope {
        SkillScope::User => user_skills_root().map_err(|error| error.to_string()),
        SkillScope::Project => {
            project_skills_root(&ctx.workspace).map_err(|error| error.to_string())
        }
    }
}

fn to_skill_scope(scope: ScopeArg) -> SkillScope {
    match scope {
        ScopeArg::User => SkillScope::User,
        ScopeArg::Project => SkillScope::Project,
    }
}

fn find_entry(
    entries: &[SkillEntry],
    name: &str,
    scope: Option<ScopeArg>,
) -> Result<SkillEntry, String> {
    let mut fallback = None;
    for entry in entries {
        if entry.name != name {
            continue;
        }
        if let Some(scope) = scope
            && entry.scope != to_skill_scope(scope)
        {
            continue;
        }
        if entry.shadowed_by.is_none() {
            return Ok(entry.clone());
        }
        fallback.get_or_insert_with(|| entry.clone());
    }
    fallback.ok_or_else(|| match scope {
        Some(scope) => format!(
            "skill '{name}' was not found in the {} scope",
            scope.as_str()
        ),
        None => format!("skill '{name}' was not found"),
    })
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

fn json_entry(entry: &SkillEntry) -> serde_json::Value {
    json!({"name": entry.name, "description": entry.description, "path": entry.path, "scope": entry.scope.as_str(), "source": entry.source.as_str(), "enabled": entry.enabled, "editable": entry.source.editable(), "shadowedBy": entry.shadowed_by})
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
) -> Result<(), String> {
    let catalog = catalog(ctx, trusted)?;
    let (mut rows, mut values) = (Vec::new(), Vec::new());
    for entry in &catalog.entries {
        if scope == ListScopeArg::User && entry.scope != SkillScope::User {
            continue;
        }
        if scope == ListScopeArg::Project && entry.scope != SkillScope::Project {
            continue;
        }
        rows.push(vec![
            entry.name.clone(),
            entry.scope.as_str().into(),
            entry.source.as_str().into(),
            if entry.enabled { "yes" } else { "no" }.into(),
            if entry.shadowed_by.is_some() {
                "yes"
            } else {
                "-"
            }
            .into(),
            truncate(&entry.description, 80),
        ]);
        values.push(json_entry(entry));
    }
    out.emit(&json!({"trusted": trusted, "skills": values}), |out, w| {
        if rows.is_empty() {
            return writeln!(w, "No skills discovered.");
        }
        out.print_table(
            w,
            &[
                "NAME",
                "SCOPE",
                "SOURCE",
                "ENABLED",
                "SHADOWED",
                "DESCRIPTION",
            ],
            &rows,
        )
    })
}

fn run_show(
    ctx: &WorkspaceContext,
    out: &Output,
    trusted: bool,
    name: &str,
    scope: Option<ScopeArg>,
) -> Result<(), String> {
    let catalog = catalog(ctx, trusted)?;
    let entry = find_entry(&catalog.entries, name, scope)?;
    let (skill_name, description, body) = crate::skills::store::read_skill_at(&entry.path)?;
    let value = json!({"name": skill_name, "description": description, "path": entry.path, "scope": entry.scope.as_str(), "source": entry.source.as_str(), "enabled": entry.enabled, "editable": entry.source.editable(), "body": body});
    out.emit(&value, |_, w| {
        writeln!(w, "{:<12}{}", "Name:", skill_name)?;
        writeln!(w, "{:<12}{}", "Path:", entry.path.display())?;
        writeln!(w, "{:<12}{}", "Scope:", entry.scope.as_str())?;
        writeln!(
            w,
            "{:<12}{}",
            "Enabled:",
            if entry.enabled { "yes" } else { "no" }
        )?;
        writeln!(
            w,
            "{:<12}{}",
            "Editable:",
            if entry.source.editable() { "yes" } else { "no" }
        )?;
        writeln!(w, "{:<12}{}", "Description:", description)?;
        writeln!(w, "\n{body}")
    })
}

fn looks_like_path(raw: &str) -> bool {
    raw.contains('/') || raw.contains('\\') || raw.ends_with(".md")
}

fn resolve_entries(
    ctx: &WorkspaceContext,
    trusted: bool,
    raw: &str,
) -> Result<Vec<SkillEntry>, String> {
    let catalog = catalog(ctx, trusted)?;
    if looks_like_path(raw) {
        let path = std::fs::canonicalize(raw)
            .map_err(|error| format!("cannot resolve skill path '{raw}': {error}"))?;
        let entries: Vec<SkillEntry> = catalog
            .entries
            .into_iter()
            .filter(|entry| entry.path == path)
            .collect();
        if entries.is_empty() {
            return Err(format!("no discovered skill lives at '{}'", path.display()));
        }
        return Ok(entries);
    }
    let entries: Vec<SkillEntry> = catalog
        .entries
        .into_iter()
        .filter(|entry| entry.name == raw)
        .collect();
    if entries.is_empty() {
        return Err(format!("skill '{raw}' was not found"));
    }
    Ok(entries)
}

fn run_toggle(
    ctx: &WorkspaceContext,
    out: &Output,
    trusted: bool,
    target: &str,
    enable: bool,
) -> Result<(), String> {
    let entries = resolve_entries(ctx, trusted, target)?;
    let config = skills_config_path().map_err(|error| error.to_string())?;
    let mut file = load_skills(&config)
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    let mut changed = Vec::new();
    for entry in &entries {
        if enable {
            file.skills.disabled.retain(|path| path != &entry.path);
            if entry.source == SkillSource::Agents && !file.skills.enabled.contains(&entry.path) {
                file.skills.enabled.push(entry.path.clone());
            }
        } else {
            file.skills.enabled.retain(|path| path != &entry.path);
            if !file.skills.disabled.contains(&entry.path) {
                file.skills.disabled.push(entry.path.clone());
            }
        }
        changed.push(entry.path.clone());
    }
    save_skills(&config, &file).map_err(|error| error.to_string())?;
    let action = if enable { "enabled" } else { "disabled" };
    out.emit(
        &json!({"action": action, "paths": changed, "count": changed.len()}),
        |_, w| writeln!(w, "Skill '{target}' {action} ({} file(s)).", changed.len()),
    )
}

fn run_create(
    ctx: &WorkspaceContext,
    out: &Output,
    trusted: bool,
    name: &str,
    description: &str,
    content: &ContentInput,
    scope: ScopeArg,
) -> Result<(), String> {
    if scope == ScopeArg::Project {
        require_trusted(ctx, trusted)?;
    }
    let skill_scope = to_skill_scope(scope);
    let root = scope_root(skill_scope, ctx)?;
    if crate::skills::find_skill_file(&root.join(name)).is_some() {
        return Err(format!(
            "skill '{name}' already exists in the {} scope; use 'lx-tui skill edit'",
            scope.as_str()
        ));
    }
    let body = content.resolve(false)?.unwrap_or_default();
    let path = crate::skills::store::write_skill(&root, name, description, &body)?;
    out.emit(
        &json!({"name": name, "scope": skill_scope.as_str(), "path": path}),
        |_, w| {
            writeln!(
                w,
                "Skill '{name}' created in the {} scope at {}.",
                skill_scope.as_str(),
                path.display()
            )
        },
    )
}

fn run_edit(
    ctx: &WorkspaceContext,
    out: &Output,
    trusted: bool,
    name: &str,
    description: Option<&str>,
    content: &ContentInput,
    scope: ScopeArg,
) -> Result<(), String> {
    if scope == ScopeArg::Project {
        require_trusted(ctx, trusted)?;
    }
    let catalog = catalog(ctx, trusted)?;
    let entry = find_entry(&catalog.entries, name, Some(scope))?;
    if !entry.source.editable() {
        return Err(format!(
            "skill '{name}' comes from '.agents' and is read-only"
        ));
    }
    let (skill_name, current_description, body) = crate::skills::store::read_skill_at(&entry.path)?;
    let description = description
        .map(str::to_string)
        .unwrap_or(current_description);
    let body = content.resolve(false)?.unwrap_or(body);
    let path = crate::skills::store::write_skill(
        &scope_root(entry.scope, ctx)?,
        &skill_name,
        &description,
        &body,
    )?;
    out.emit(
        &json!({"name": skill_name, "scope": entry.scope.as_str(), "path": path}),
        |_, w| {
            writeln!(
                w,
                "Skill '{skill_name}' updated in the {} scope at {}.",
                entry.scope.as_str(),
                path.display()
            )
        },
    )
}

fn run_delete(
    ctx: &WorkspaceContext,
    out: &Output,
    trusted: bool,
    name: &str,
    scope: ScopeArg,
) -> Result<(), String> {
    if scope == ScopeArg::Project {
        require_trusted(ctx, trusted)?;
    }
    let catalog = catalog(ctx, trusted)?;
    let entry = find_entry(&catalog.entries, name, Some(scope))?;
    if !entry.source.editable() {
        return Err(format!(
            "skill '{name}' comes from '.agents' and is read-only"
        ));
    }
    crate::skills::store::delete_skill(&scope_root(entry.scope, ctx)?, &entry.name)?;
    out.emit(
        &json!({"name": entry.name, "scope": entry.scope.as_str()}),
        |_, w| {
            writeln!(
                w,
                "Skill '{}' deleted from the {} scope.",
                entry.name,
                entry.scope.as_str()
            )
        },
    )
}
