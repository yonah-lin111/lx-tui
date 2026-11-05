//! `lx-tui subagent` 子命令执行与角色配置读写。

use super::*;

pub fn run(
    cli: SubagentCli,
    linked_worktree: Option<&Path>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let ctx = WorkspaceContext::resolve(cli.global.cwd.as_deref(), linked_worktree)?;
    let out = Output::new(cli.global.json, cli.global.color);
    let trusted = is_trusted(&ctx)?;
    let base = load_section()?;
    let merged =
        merge_project_roles_for_paths(&base, Some(&ctx.workspace), Some(&ctx.cwd), trusted);
    match cli.command {
        SubagentCommand::Show { role, scope } => run_show(&out, &merged, role.as_deref(), scope)?,
        SubagentCommand::Set(args) => run_set(&out, &args)?,
        SubagentCommand::RoleSet(args) => run_role_set(&ctx, &out, trusted, &base, &merged, &args)?,
        SubagentCommand::RoleReset { name } => run_role_reset(&out, &name)?,
        SubagentCommand::RoleRm { name, scope } => run_role_rm(&ctx, &out, trusted, &name, scope)?,
    }
    Ok(EXIT_OK)
}
fn is_trusted(ctx: &WorkspaceContext) -> Result<bool, String> {
    ctx.open_db()?
        .is_trusted(&ctx.workspace)
        .map_err(|error| error.code().to_string())
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
fn load_section() -> Result<SubagentsSection, String> {
    let path = subagents_config_path().map_err(|error| error.to_string())?;
    Ok(load_subagents(&path)
        .map_err(|error| error.to_string())?
        .map(|file| file.subagents)
        .unwrap_or_default())
}
fn save_section(section: &SubagentsSection) -> Result<(), String> {
    let path = subagents_config_path().map_err(|error| error.to_string())?;
    save_subagents(
        &path,
        &SubagentsConfigFile {
            subagents: section.clone(),
        },
    )
    .map_err(|error| error.to_string())
}
fn format_model(selection: Option<&ModelSelection>) -> String {
    match selection {
        Some(sel) => match &sel.variant {
            Some(variant) => format!("{}/{}#{variant}", sel.provider, sel.model),
            None => format!("{}/{}", sel.provider, sel.model),
        },
        None => "-".to_string(),
    }
}
fn scope_label(role: &ResolvedRoleSpec) -> &'static str {
    if role.is_builtin {
        "builtin"
    } else if role.is_project_scoped {
        "project"
    } else {
        "user"
    }
}
fn role_value(role: &ResolvedRoleSpec) -> serde_json::Value {
    json!({"name": role.name, "baseRole": role.base_role.as_str(), "builtin": role.is_builtin, "projectScoped": role.is_project_scoped, "description": role.description, "systemPrompt": role.system_prompt_override, "model": role.default_model.as_ref().map(|sel| json!({"provider": sel.provider, "model": sel.model, "variant": sel.variant})), "lockModel": role.lock_model, "capabilities": {"allowWebSearch": role.capabilities.allow_web_search, "allowMcp": role.capabilities.allow_mcp, "allowSkills": role.capabilities.allow_skills}, "nicknameCandidates": role.nickname_candidates})
}
fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
fn run_show(
    out: &Output,
    section: &SubagentsSection,
    name: Option<&str>,
    scope: ListScopeArg,
) -> Result<(), String> {
    if let Some(name) = name {
        let role = section
            .resolve_role(name)
            .ok_or_else(|| format!("subagent role '{name}' is not configured"))?;
        if scope == ListScopeArg::Project && !role.is_project_scoped {
            return Err(format!("subagent role '{name}' is not project-scoped"));
        }
        if scope == ListScopeArg::User && role.is_project_scoped {
            return Err(format!("subagent role '{name}' is project-scoped"));
        }
        out.emit(&role_value(&role), |_, w| {
            writeln!(w, "{:<18}{}", "Role:", role.name)?;
            writeln!(w, "{:<18}{}", "Base role:", role.base_role.as_str())?;
            writeln!(w, "{:<18}{}", "Scope:", scope_label(&role))?;
            writeln!(
                w,
                "{:<18}{}",
                "Model:",
                format_model(role.default_model.as_ref())
            )?;
            writeln!(w, "{:<18}{}", "Lock model:", yes_no(role.lock_model))?;
            writeln!(
                w,
                "{:<18}web_search={} mcp={} skills={}",
                "Capabilities:",
                yes_no(role.capabilities.allow_web_search),
                yes_no(role.capabilities.allow_mcp),
                yes_no(role.capabilities.allow_skills)
            )?;
            writeln!(w, "{:<18}{}", "Description:", role.description)?;
            if let Some(prompt) = &role.system_prompt_override {
                writeln!(w, "\n{prompt}")?;
            }
            Ok(())
        })
    } else {
        let roles: Vec<ResolvedRoleSpec> = section
            .all_roles()
            .into_iter()
            .filter(|role| match scope {
                ListScopeArg::All => true,
                ListScopeArg::User => !role.is_project_scoped,
                ListScopeArg::Project => role.is_project_scoped,
            })
            .collect();
        out.emit(&json!({"maxConcurrent": section.max_concurrent, "maxDepth": section.max_depth, "defaultMaxTokens": section.default_max_tokens, "delegationMode": section.delegation_mode.as_str(), "roles": roles.iter().map(role_value).collect::<Vec<_>>()}), |out, w| {
            writeln!(w, "{:<20}{}", "Max concurrent:", section.max_concurrent)?;
            writeln!(w, "{:<20}{}", "Max depth:", section.max_depth)?;
            writeln!(w, "{:<20}{}", "Default max tokens:", section.default_max_tokens.map(|value| value.to_string()).unwrap_or_else(|| "unlimited".to_string()))?;
            writeln!(w, "{:<20}{}", "Delegation mode:", section.delegation_mode.as_str())?;
            let rows: Vec<Vec<String>> = roles.iter().map(|role| vec![role.name.clone(), role.base_role.as_str().into(), scope_label(role).into(), format_model(role.default_model.as_ref()), yes_no(role.lock_model).into(), yes_no(role.capabilities.allow_web_search).into(), yes_no(role.capabilities.allow_mcp).into(), yes_no(role.capabilities.allow_skills).into()]).collect();
            writeln!(w)?;
            out.print_table(w, &["NAME", "BASE", "SCOPE", "MODEL", "LOCK", "WEB", "MCP", "SKILLS"], &rows)?;
            for role in &roles { writeln!(w, "  {}: {}", role.name, role.description)?; }
            Ok(())
        })
    }
}
fn run_set(out: &Output, args: &SetArgs) -> Result<(), String> {
    let mut section = load_section()?;
    if let Some(value) = args.max_concurrent {
        section.max_concurrent = value;
    }
    if let Some(value) = args.max_depth {
        section.max_depth = value;
    }
    if args.clear_max_tokens {
        section.default_max_tokens = None;
    } else if let Some(value) = args.default_max_tokens {
        section.default_max_tokens = Some(value);
    }
    if let Some(mode) = args.delegation_mode {
        section.delegation_mode = mode;
    }
    save_section(&section)?;
    out.emit(&json!({"maxConcurrent": section.max_concurrent, "maxDepth": section.max_depth, "defaultMaxTokens": section.default_max_tokens, "delegationMode": section.delegation_mode.as_str()}), |_, w| writeln!(w, "Subagent settings saved (max concurrent {}, max depth {}, delegation {}).", section.max_concurrent, section.max_depth, section.delegation_mode.as_str()))
}
fn apply_capabilities(caps: &mut RoleCapabilities, args: &RoleSetArgs) {
    if let Some(value) = args.allow_web_search {
        caps.allow_web_search = value;
    }
    if let Some(value) = args.allow_mcp {
        caps.allow_mcp = value;
    }
    if let Some(value) = args.allow_skills {
        caps.allow_skills = value;
    }
}
fn apply_model(current: Option<ModelSelection>, change: &ModelChange) -> Option<ModelSelection> {
    match change {
        ModelChange::Keep => current,
        ModelChange::Clear => None,
        ModelChange::Set(selection) => Some(selection.clone()),
    }
}
fn apply_override(role: &mut RoleOverrideConfig, args: &RoleSetArgs, prompt: &Option<String>) {
    if let Some(description) = &args.description {
        role.description = Some(description.clone());
    }
    if let Some(prompt) = prompt {
        role.system_prompt = Some(prompt.clone());
    }
    role.model = apply_model(role.model.clone(), &args.model);
    if let Some(value) = args.lock_model {
        role.lock_model = value;
    }
    apply_capabilities(&mut role.capabilities, args);
    if let Some(nicknames) = &args.nicknames {
        role.nickname_candidates = nicknames.clone();
    }
}
fn apply_custom(role: &mut CustomRoleConfig, args: &RoleSetArgs, prompt: &Option<String>) {
    if let Some(base) = args.base {
        role.base_role = base;
    }
    if let Some(description) = &args.description {
        role.description = description.clone();
    }
    if let Some(prompt) = prompt {
        role.system_prompt = prompt.clone();
    }
    role.model = apply_model(role.model.clone(), &args.model);
    if let Some(value) = args.lock_model {
        role.lock_model = value;
    }
    apply_capabilities(&mut role.capabilities, args);
    if let Some(nicknames) = &args.nicknames {
        role.nickname_candidates = nicknames.clone();
    }
}
fn builtin_override_mut(section: &mut SubagentsSection, base: BaseRole) -> &mut RoleOverrideConfig {
    match base {
        BaseRole::Explorer => &mut section.builtins.explorer,
        BaseRole::Executor => &mut section.builtins.executor,
        BaseRole::Monitor => &mut section.builtins.monitor,
    }
}
fn run_role_set(
    ctx: &WorkspaceContext,
    out: &Output,
    trusted: bool,
    base: &SubagentsSection,
    merged: &SubagentsSection,
    args: &RoleSetArgs,
) -> Result<(), String> {
    let prompt = if args.prompt.inline.is_some() || args.prompt.file.is_some() {
        args.prompt.resolve(true)?
    } else {
        None
    };
    if let Some(base_role) = BaseRole::parse(&args.name) {
        if args.scope == ScopeArg::Project {
            return Err(format!(
                "built-in role '{}' can only be overridden in the user scope",
                args.name
            ));
        }
        if args.base.is_some() {
            return Err("'--base' cannot be used with a built-in role".into());
        }
        let mut section = base.clone();
        apply_override(builtin_override_mut(&mut section, base_role), args, &prompt);
        save_section(&section)?;
        return out.emit(&json!({"role": args.name, "scope": "builtin"}), |_, w| {
            writeln!(w, "Built-in role '{}' overrides saved.", args.name)
        });
    }
    if !is_valid_role_name(&args.name) {
        return Err(format!(
            "invalid role name '{}'; expected [a-z][a-z0-9_]{{0,31}}",
            args.name
        ));
    }
    if args.scope == ScopeArg::Project {
        require_trusted(ctx, trusted)?;
        if base.roles.iter().any(|role| role.name == args.name) {
            return Err(format!(
                "custom role '{}' already exists in the user scope; use '--scope user' to edit it",
                args.name
            ));
        }
        let mut role = match read_project_role(&ctx.workspace, &args.name)? {
            Some(role) => role,
            None => merged
                .resolve_role(&args.name)
                .filter(|role| role.is_project_scoped)
                .map(|resolved| CustomRoleConfig {
                    name: resolved.name,
                    base_role: resolved.base_role,
                    description: resolved.description,
                    system_prompt: resolved.system_prompt_override.unwrap_or_default(),
                    model: resolved.default_model,
                    lock_model: resolved.lock_model,
                    capabilities: resolved.capabilities,
                    nickname_candidates: resolved.nickname_candidates,
                    is_project_scoped: true,
                })
                .unwrap_or_default(),
        };
        role.name = args.name.clone();
        role.is_project_scoped = true;
        apply_custom(&mut role, args, &prompt);
        if role.description.trim().is_empty()
            || role.description.chars().count() > MAX_ROLE_DESCRIPTION_CHARS
            || role.system_prompt.trim().is_empty()
            || role.system_prompt.chars().count() > MAX_ROLE_PROMPT_CHARS
        {
            return Err(format!(
                "custom role '{}' requires a valid '--description' and '--prompt'",
                args.name
            ));
        }
        let path = write_project_role(&ctx.workspace, &role)?;
        return out.emit(
            &json!({"role": args.name, "scope": "project", "path": path}),
            |_, w| {
                writeln!(
                    w,
                    "Custom role '{}' written to {}.",
                    args.name,
                    path.display()
                )
            },
        );
    }
    let mut section = base.clone();
    let position = section.roles.iter().position(|role| role.name == args.name);
    let mut role = position
        .and_then(|index| section.roles.get(index).cloned())
        .unwrap_or_default();
    role.name = args.name.clone();
    role.is_project_scoped = false;
    apply_custom(&mut role, args, &prompt);
    if role.description.trim().is_empty()
        || role.description.chars().count() > MAX_ROLE_DESCRIPTION_CHARS
        || role.system_prompt.trim().is_empty()
        || role.system_prompt.chars().count() > MAX_ROLE_PROMPT_CHARS
    {
        return Err(format!(
            "custom role '{}' requires a valid '--description' and '--prompt'",
            args.name
        ));
    }
    match position {
        Some(index) => section.roles[index] = role,
        None => section.roles.push(role),
    }
    save_section(&section)?;
    out.emit(&json!({"role": args.name, "scope": "user"}), |_, w| {
        writeln!(w, "Custom role '{}' saved in the user scope.", args.name)
    })
}
fn run_role_reset(out: &Output, name: &str) -> Result<(), String> {
    let base = BaseRole::parse(name).ok_or_else(|| {
        format!("unknown built-in role '{name}'; expected 'explorer', 'executor', or 'monitor'")
    })?;
    let mut section = load_section()?;
    *builtin_override_mut(&mut section, base) = RoleOverrideConfig::default();
    save_section(&section)?;
    out.emit(&json!({"role": base.as_str(), "reset": true}), |_, w| {
        writeln!(w, "Built-in role '{}' reset to defaults.", base.as_str())
    })
}
fn run_role_rm(
    ctx: &WorkspaceContext,
    out: &Output,
    trusted: bool,
    name: &str,
    scope: ScopeArg,
) -> Result<(), String> {
    if BaseRole::parse(name).is_some() {
        return Err(format!(
            "built-in role '{name}' cannot be removed; use 'lx-tui subagent role-reset'"
        ));
    }
    if !is_valid_role_name(name) {
        return Err(format!("invalid role name '{name}'"));
    }
    if scope == ScopeArg::User {
        let mut section = load_section()?;
        let before = section.roles.len();
        section.roles.retain(|role| role.name != name);
        if section.roles.len() == before {
            return Err(format!(
                "custom role '{name}' was not found in the user scope"
            ));
        }
        save_section(&section)?;
    } else {
        require_trusted(ctx, trusted)?;
        let mut removed = false;
        for path in [
            ctx.workspace
                .join(".lx/agents")
                .join(format!("{name}.json")),
            ctx.workspace
                .join(".agents/roles")
                .join(format!("{name}.json")),
        ] {
            if path.is_file() {
                std::fs::remove_file(&path)
                    .map_err(|error| format!("failed to remove '{}': {error}", path.display()))?;
                removed = true;
            }
        }
        if !removed {
            return Err(format!(
                "project role '{name}' was not found under '{}'",
                ctx.workspace.display()
            ));
        }
    }
    out.emit(
        &json!({"removed": name, "scope": scope.as_str()}),
        |_, w| {
            writeln!(
                w,
                "Custom role '{name}' removed from the {} scope.",
                scope.as_str()
            )
        },
    )
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectRoleDraft {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    base_role: BaseRole,
    description: String,
    system_prompt: String,
    #[serde(default)]
    model: Option<ModelSelection>,
    #[serde(default)]
    lock_model: bool,
    #[serde(default, flatten)]
    capabilities: RoleCapabilities,
    #[serde(default)]
    nickname_candidates: Vec<String>,
}
fn project_role_path(workspace: &Path, name: &str) -> PathBuf {
    workspace
        .join(".lx")
        .join("agents")
        .join(format!("{name}.json"))
}
fn read_project_role(workspace: &Path, name: &str) -> Result<Option<CustomRoleConfig>, String> {
    let path = project_role_path(workspace, name);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to read '{}': {error}", path.display())),
    };
    let draft: ProjectRoleDraft = serde_json::from_str(&text)
        .map_err(|error| format!("failed to parse '{}': {error}", path.display()))?;
    Ok(Some(CustomRoleConfig {
        name: draft.name.unwrap_or_else(|| name.to_string()),
        base_role: draft.base_role,
        description: draft.description,
        system_prompt: draft.system_prompt,
        model: draft.model,
        lock_model: draft.lock_model,
        capabilities: draft.capabilities,
        nickname_candidates: draft.nickname_candidates,
        is_project_scoped: true,
    }))
}
fn write_project_role(workspace: &Path, role: &CustomRoleConfig) -> Result<PathBuf, String> {
    let dir = workspace.join(".lx").join("agents");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create '{}': {error}", dir.display()))?;
    crate::platform::restrict_dir_permissions(&dir)
        .map_err(|error| format!("failed to restrict '{}': {error}", dir.display()))?;
    let body = serde_json::to_string_pretty(role)
        .map_err(|error| format!("failed to serialize project role: {error}"))?;
    let path = project_role_path(workspace, &role.name);
    crate::platform::write_file_atomic(&path, format!("{body}\n").as_bytes())
        .map_err(|error| format!("failed to write '{}': {error}", path.display()))?;
    crate::platform::restrict_file_permissions(&path)
        .map_err(|error| format!("failed to restrict '{}': {error}", path.display()))?;
    Ok(path)
}
