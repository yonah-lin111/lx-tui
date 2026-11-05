//! `lx-tui prompt` 子命令执行与输出格式化。

use super::*;

pub fn run(
    cli: PromptCli,
    linked_worktree: Option<&Path>,
) -> Result<i32, Box<dyn std::error::Error>> {
    execute(cli, linked_worktree).map_err(Into::into)
}

fn execute(cli: PromptCli, linked_worktree: Option<&Path>) -> Result<i32, String> {
    let workspace = WorkspaceContext::resolve(cli.global.cwd.as_deref(), linked_worktree)?;
    let ctx = Context {
        db: workspace.open_db()?,
        cwd: workspace.cwd,
        ws: workspace.workspace,
        out: Output::new(cli.global.json, cli.global.color),
    };
    let handler = COMMANDS
        .iter()
        .find(|(name, _, _)| *name == cli.command.action)
        .map(|(_, _, handler)| *handler)
        .ok_or_else(|| format!("unknown prompt command '{}'", cli.command.action))?;
    handler(&ctx, &cli.command)?;
    Ok(EXIT_OK)
}

pub(super) struct Context {
    db: Database,
    cwd: PathBuf,
    ws: PathBuf,
    out: Output,
}

pub(super) fn list(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let mut folders = load_folders(ctx)?;
    let mut prompts = load_prompts(ctx)?;
    if let Some(id) = command.folder {
        ensure_folder(ctx, id)?;
        folders.retain(|item| item.id == id || item.parent_id == Some(id));
        prompts.retain(|prompt| prompt.folder_id == Some(id));
    }
    let payload = json!({
        "workspace": ctx.ws.display().to_string(),
        "folders": folders_json(&folders),
        "prompts": prompts_json(&prompts, &folders),
    });
    ctx.out.emit(&payload, |out, writer| {
        if folders.is_empty() && prompts.is_empty() {
            return writeln!(writer, "No prompts or folders found.");
        }
        if !folders.is_empty() {
            writeln!(writer, "{}", out.bold("Folders"))?;
            writeln!(writer, "{}", out.bold("ID  PARENT  NAME"))?;
            for folder in &folders {
                writeln!(
                    writer,
                    "{}  {}  {}",
                    folder.id,
                    folder
                        .parent_id
                        .map_or_else(|| "-".to_string(), |id| id.to_string()),
                    folder.name
                )?;
            }
        }
        if !prompts.is_empty() {
            if !folders.is_empty() {
                writeln!(writer)?;
            }
            writeln!(writer, "{}", out.bold("Prompts"))?;
            writeln!(writer, "{}", out.bold("ID  FOLDER  NAME  PAGES  ACTIVE"))?;
            for prompt in &prompts {
                writeln!(
                    writer,
                    "{}  {}  {}  {}  {}",
                    prompt.id,
                    folder_name(&folders, prompt.folder_id).unwrap_or_else(|| "(root)".to_string()),
                    prompt.name,
                    prompt.pages.len(),
                    prompt
                        .active_page_id
                        .map_or_else(|| "-".to_string(), |id| id.to_string())
                )?;
            }
        }
        Ok(())
    })
}

pub(super) fn show(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let prompt_id = command.id(0, "PROMPT_ID")?;
    let folders = load_folders(ctx)?;
    let prompts = load_prompts(ctx)?;
    let prompt = find_prompt(&prompts, prompt_id)?;
    let selected: Vec<&PromptPageRecord> = match command.page.as_deref() {
        Some(raw) => vec![resolve_page(prompt, raw)?],
        None => prompt.pages.iter().collect(),
    };
    let display = folder_name(&folders, prompt.folder_id);
    let payload = json!({
        "id": prompt.id, "name": prompt.name, "folder_id": prompt.folder_id,
        "folder_name": display, "active_page_id": prompt.active_page_id,
        "page_count": prompt.pages.len(), "pages": pages_json(prompt, &selected),
    });
    ctx.out.emit(&payload, |out, writer| {
        writeln!(
            writer,
            "{}",
            out.bold(&format!("Prompt: {} (id={})", prompt.name, prompt.id))
        )?;
        writeln!(writer, "Folder: {}", display.as_deref().unwrap_or("(root)"))?;
        for page in &selected {
            writeln!(writer)?;
            let marker = if prompt.active_page_id == Some(page.id) {
                " [active]"
            } else {
                ""
            };
            writeln!(
                writer,
                "{}",
                out.cyan(&format!(
                    "[{}] {} (id={}){marker}",
                    page.position + 1,
                    page.name,
                    page.id
                ))
            )?;
            write!(writer, "{}", page.content)?;
            if !page.content.ends_with('\n') {
                writeln!(writer)?;
            }
        }
        Ok(())
    })
}

pub(super) fn create(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let name = command.word(0, "NAME")?;
    let content = command.content.resolve(false)?.unwrap_or_default();
    if let Some(folder) = command.folder {
        ensure_folder(ctx, folder)?;
    }
    db(
        ctx.db.ensure_workspace(&ctx.ws),
        "failed to register workspace",
    )?;
    let id = db(
        ctx.db.create_prompt(&ctx.ws, command.folder, &name, &[]),
        "failed to create prompt",
    )?;
    let prompts = load_prompts(ctx)?;
    let first = find_prompt(&prompts, id)?
        .pages
        .first()
        .ok_or_else(|| format!("prompt {id} was created without pages"))?;
    let page_name = command
        .page_name
        .clone()
        .unwrap_or_else(|| "Page 1".to_string());
    if page_name != "Page 1" {
        db(
            ctx.db.rename_prompt_page(first.id, &page_name),
            "failed to rename first page",
        )?;
    }
    if !content.is_empty() {
        db(
            ctx.db.update_page_content(first.id, &content),
            "failed to write page content",
        )?;
    }
    action(
        &ctx.out,
        "create",
        id,
        format!(
            "Created prompt {id} ('{name}') with page {} ('{page_name}').",
            first.id
        ),
    )
}

pub(super) fn edit(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let prompt_id = command.id(0, "PROMPT_ID")?;
    let prompts = load_prompts(ctx)?;
    let prompt = find_prompt(&prompts, prompt_id)?;
    let page = match command.page.as_deref() {
        Some(raw) => page_by_id(prompt, parse_id(raw, "--page")?)?,
        None => active_page(prompt)?,
    };
    let Some(text) = command.content.resolve(true)? else {
        return Err("missing content".to_string());
    };
    db(
        ctx.db.update_page_content(page.id, &text),
        "failed to update page",
    )?;
    action(
        &ctx.out,
        "edit",
        prompt_id,
        format!("Updated page {} of prompt {prompt_id}.", page.id),
    )
}

pub(super) fn rename_prompt(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let id = command.id(0, "PROMPT_ID")?;
    ensure_prompt(ctx, id)?;
    db(
        ctx.db.rename_prompt(id, &command.word(1, "NEW_NAME")?),
        "failed to rename prompt",
    )?;
    action(&ctx.out, "rename", id, format!("Renamed prompt {id}."))
}

pub(super) fn move_prompt(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let id = command.id(0, "PROMPT_ID")?;
    ensure_prompt(ctx, id)?;
    let folder = command.destination("--folder")?;
    db(ctx.db.move_prompt(id, folder), "failed to move prompt")?;
    action(&ctx.out, "move", id, format!("Moved prompt {id}."))
}

pub(super) fn delete_prompt(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let id = command.id(0, "PROMPT_ID")?;
    ensure_prompt(ctx, id)?;
    db(ctx.db.delete_prompt(id), "failed to delete prompt")?;
    action(&ctx.out, "delete", id, format!("Deleted prompt {id}."))
}

pub(super) fn folder_list(ctx: &Context, _command: &PromptCommand) -> Result<(), String> {
    let folders = load_folders(ctx)?;
    let payload = json!({ "folders": folders_json(&folders) });
    ctx.out.emit(&payload, |out, writer| {
        if folders.is_empty() {
            return writeln!(writer, "No folders found.");
        }
        writeln!(writer, "{}", out.bold("ID  PARENT  NAME"))?;
        for folder in &folders {
            writeln!(
                writer,
                "{}  {}  {}",
                folder.id,
                folder
                    .parent_id
                    .map_or_else(|| "-".to_string(), |id| id.to_string()),
                folder.name
            )?;
        }
        Ok(())
    })
}

pub(super) fn folder_create(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    db(
        ctx.db.ensure_workspace(&ctx.ws),
        "failed to register workspace",
    )?;
    let name = command.word(0, "NAME")?;
    let id = db(
        ctx.db.create_folder(&ctx.ws, command.parent, &name),
        "failed to create folder",
    )?;
    action(&ctx.out, "create", id, format!("Created folder {id}."))
}

pub(super) fn folder_rename(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let id = command.id(0, "FOLDER_ID")?;
    ensure_folder(ctx, id)?;
    db(
        ctx.db.rename_folder(id, &command.word(1, "NEW_NAME")?),
        "failed to rename folder",
    )?;
    action(&ctx.out, "rename", id, format!("Renamed folder {id}."))
}

pub(super) fn folder_move(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let id = command.id(0, "FOLDER_ID")?;
    ensure_folder(ctx, id)?;
    let parent = command.destination("--parent")?;
    db(ctx.db.move_folder(id, parent), "failed to move folder")?;
    action(&ctx.out, "move", id, format!("Moved folder {id}."))
}

pub(super) fn folder_delete(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let id = command.id(0, "FOLDER_ID")?;
    ensure_folder(ctx, id)?;
    db(ctx.db.delete_folder(id), "failed to delete folder")?;
    action(&ctx.out, "delete", id, format!("Deleted folder {id}."))
}

pub(super) fn page_list(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let prompt_id = command.id(0, "PROMPT_ID")?;
    let prompts = load_prompts(ctx)?;
    let prompt = find_prompt(&prompts, prompt_id)?;
    let payload = json!({
        "prompt_id": prompt_id, "active_page_id": prompt.active_page_id,
        "pages": pages_json(prompt, &prompt.pages.iter().collect::<Vec<_>>()),
    });
    ctx.out.emit(&payload, |out, writer| {
        if prompt.pages.is_empty() {
            return writeln!(writer, "No pages found.");
        }
        writeln!(writer, "{}", out.bold("ID  #  NAME  ACTIVE  CHARS"))?;
        for page in &prompt.pages {
            writeln!(
                writer,
                "{}  {}  {}  {}  {}",
                page.id,
                page.position + 1,
                page.name,
                if prompt.active_page_id == Some(page.id) {
                    "yes"
                } else {
                    "no"
                },
                page.content.chars().count()
            )?;
        }
        Ok(())
    })
}

pub(super) fn page_create(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let prompt_id = command.id(0, "PROMPT_ID")?;
    ensure_prompt(ctx, prompt_id)?;
    let content = command.content.resolve(false)?.unwrap_or_default();
    let name = command.word(1, "PAGE_NAME")?;
    let id = db(
        ctx.db.create_prompt_page(prompt_id, &name, &content),
        "failed to create page",
    )?;
    action(
        &ctx.out,
        "create",
        id,
        format!("Created page {id} in prompt {prompt_id}."),
    )
}

pub(super) fn page_edit(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let id = command.id(0, "PAGE_ID")?;
    let Some(text) = command.content.resolve(true)? else {
        return Err("missing content".to_string());
    };
    db(
        ctx.db.update_page_content(id, &text),
        "failed to update page",
    )?;
    action(&ctx.out, "edit", id, format!("Updated page {id}."))
}

pub(super) fn page_rename(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let id = command.id(0, "PAGE_ID")?;
    db(
        ctx.db.rename_prompt_page(id, &command.word(1, "NEW_NAME")?),
        "failed to rename page",
    )?;
    action(&ctx.out, "rename", id, format!("Renamed page {id}."))
}

pub(super) fn page_active(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let prompt_id = command.id(0, "PROMPT_ID")?;
    let page_id = command.id(1, "PAGE_ID")?;
    db(
        ctx.db.set_prompt_active_page(prompt_id, page_id),
        "failed to set active page",
    )?;
    action(
        &ctx.out,
        "active",
        page_id,
        format!("Set page {page_id} as the active page of prompt {prompt_id}."),
    )
}

pub(super) fn page_delete(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let id = command.id(0, "PAGE_ID")?;
    let prompts = db(ctx.db.list_prompts(&ctx.ws), "failed to list prompts")?;
    if let Some(prompt) = prompts
        .iter()
        .find(|p| p.pages.iter().any(|page| page.id == id))
        && prompt.pages.len() <= 1
    {
        return Err(format!(
            "cannot delete page {id}: prompt {} must keep at least one page",
            prompt.id
        ));
    }
    let next = db(ctx.db.delete_prompt_page(id), "failed to delete page")?;
    let message = next.map_or_else(
        || format!("Deleted page {id}."),
        |next| format!("Deleted page {id}; active page is now {next}."),
    );
    action(&ctx.out, "delete", id, message)
}

pub(super) fn ref_list(ctx: &Context, _command: &PromptCommand) -> Result<(), String> {
    let records = db(
        ctx.db.list_referenced_folders(&ctx.ws),
        "failed to list referenced folders",
    )?;
    let payload = json!({
        "folders": records.iter().map(|record| json!({"id": record.id, "path": record.path})).collect::<Vec<_>>(),
    });
    ctx.out.emit(&payload, |out, writer| {
        if records.is_empty() {
            return writeln!(writer, "No referenced folders found.");
        }
        writeln!(writer, "{}", out.bold("ID  PATH"))?;
        for record in &records {
            writeln!(writer, "{}  {}", record.id, record.path)?;
        }
        Ok(())
    })
}

pub(super) fn ref_add(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let path = PathBuf::from(command.word(0, "DIR_PATH")?);
    let path = if path.is_absolute() {
        path
    } else {
        ctx.cwd.join(path)
    };
    let absolute = std::fs::canonicalize(&path)
        .map_err(|error| format!("failed to resolve directory '{}': {error}", path.display()))?;
    if !absolute.is_dir() {
        return Err(format!("'{}' is not a directory", absolute.display()));
    }
    let value = absolute.to_string_lossy().into_owned();
    db(
        ctx.db.ensure_workspace(&ctx.ws),
        "failed to register workspace",
    )?;
    let id = db(
        ctx.db.add_referenced_folder(&ctx.ws, &value),
        "failed to add referenced folder",
    )?;
    action(
        &ctx.out,
        "add",
        id,
        format!("Referenced folder {id}: {value}"),
    )
}

pub(super) fn ref_remove(ctx: &Context, command: &PromptCommand) -> Result<(), String> {
    let id = command.id(0, "REF_ID")?;
    if db(
        ctx.db.list_referenced_folders(&ctx.ws),
        "failed to list referenced folders",
    )?
    .iter()
    .all(|record| record.id != id)
    {
        return Err(format!("referenced folder {id} not found"));
    }
    db(
        ctx.db.delete_referenced_folder(&ctx.ws, id),
        "failed to remove referenced folder",
    )?;
    action(
        &ctx.out,
        "remove",
        id,
        format!("Removed referenced folder {id}."),
    )
}

fn action(out: &Output, name: &str, id: i64, message: String) -> Result<(), String> {
    let payload = json!({ "action": name, "id": id, "message": message.clone() });
    out.emit(&payload, |_, writer| writeln!(writer, "{message}"))
}

pub(super) fn load_folders(ctx: &Context) -> Result<Vec<FolderRecord>, String> {
    db(ctx.db.list_folders(&ctx.ws), "failed to list folders")
}

fn db<T>(result: crate::db::Result<T>, context: &str) -> Result<T, String> {
    result.map_err(|error| format!("{context}: {}", error.code()))
}

pub(super) fn load_prompts(ctx: &Context) -> Result<Vec<PromptRecord>, String> {
    db(ctx.db.list_prompts(&ctx.ws), "failed to list prompts")
}

pub(super) fn ensure_folder(ctx: &Context, id: i64) -> Result<(), String> {
    load_folders(ctx)?
        .iter()
        .any(|folder| folder.id == id)
        .then_some(())
        .ok_or_else(|| format!("folder {id} not found"))
}

pub(super) fn ensure_prompt(ctx: &Context, id: i64) -> Result<(), String> {
    load_prompts(ctx)?
        .iter()
        .any(|prompt| prompt.id == id)
        .then_some(())
        .ok_or_else(|| format!("prompt {id} not found"))
}

fn folders_json(folders: &[FolderRecord]) -> Vec<Value> {
    folders
        .iter()
        .map(|folder| json!({"id": folder.id, "parent_id": folder.parent_id, "name": folder.name}))
        .collect()
}

fn prompts_json(prompts: &[PromptRecord], folders: &[FolderRecord]) -> Vec<Value> {
    prompts
        .iter()
        .map(|prompt| {
            json!({
                "id": prompt.id, "name": prompt.name, "folder_id": prompt.folder_id,
                "folder_name": folder_name(folders, prompt.folder_id),
                "page_count": prompt.pages.len(), "active_page_id": prompt.active_page_id,
            })
        })
        .collect()
}

fn pages_json(prompt: &PromptRecord, pages: &[&PromptPageRecord]) -> Vec<Value> {
    pages
        .iter()
        .map(|page| {
            json!({
                "id": page.id, "position": page.position, "name": page.name,
                "content": page.content, "active": prompt.active_page_id == Some(page.id),
            })
        })
        .collect()
}

fn folder_name(folders: &[FolderRecord], id: Option<i64>) -> Option<String> {
    id.and_then(|id| {
        folders
            .iter()
            .find(|folder| folder.id == id)
            .map(|folder| folder.name.clone())
    })
}

fn find_prompt(prompts: &[PromptRecord], id: i64) -> Result<&PromptRecord, String> {
    prompts
        .iter()
        .find(|prompt| prompt.id == id)
        .ok_or_else(|| format!("prompt {id} not found"))
}
fn active_page(prompt: &PromptRecord) -> Result<&PromptPageRecord, String> {
    prompt
        .active_page_id
        .and_then(|id| prompt.pages.iter().find(|page| page.id == id))
        .or_else(|| prompt.pages.first())
        .ok_or_else(|| format!("prompt {} has no pages", prompt.id))
}

fn page_by_id(prompt: &PromptRecord, id: i64) -> Result<&PromptPageRecord, String> {
    prompt
        .pages
        .iter()
        .find(|page| page.id == id)
        .ok_or_else(|| format!("page {id} not found"))
}

fn resolve_page<'a>(prompt: &'a PromptRecord, raw: &str) -> Result<&'a PromptPageRecord, String> {
    let value = raw.trim().parse::<i64>().ok();
    let page = value.and_then(|value| {
        prompt
            .pages
            .iter()
            .find(|page| page.id == value)
            .or_else(|| {
                usize::try_from(value)
                    .ok()
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| prompt.pages.get(index))
            })
    });
    page.ok_or_else(|| {
        format!(
            "page '{raw}' not found in prompt {} ({} pages)",
            prompt.id,
            prompt.pages.len()
        )
    })
}
