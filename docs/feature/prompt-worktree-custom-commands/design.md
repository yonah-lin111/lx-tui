# Prompt worktree and custom commands - design

- Status: implemented (on `feat/prompt-worktree-custom-commands`); post-implementation review adjustments applied: the settings button is the ASCII label `[SET]` and the tab-bar exit button `[EXIT]`, the Commands page is a dark list/editor layout with row selection and an empty placeholder (no preview pane, no per-row edit button), source tags render `[Built-in]`/`[User]`/`[Project]`/`[wt]`, the template title line uses `{title: …}` metadata syntax with the `{title:` prefix and closing `}` protected (only the value is editable) and one blank line after it, the binding lives on its own protected line at the bottom of the block (the line above the end line) with one blank line above it (both blanks render as block-border lines and carry no document text), and for a linked-worktree binding the `@` panel first asks for the scan root (`main` / `wt`) and appends `-[main]` / `-[wt]` to the panel title instead of tagging individual rows.
- References: `lx-agent/src/renderer/src/features/markdown/LxMarkdownEditor.tsx`, its slash command and custom command services.

## Goal

Add `/gitWorktree` inside `&&&` template blocks, editable Markdown slash commands, a small English settings Web UI, and bracketed source tags in the prompt command panel. Preserve the TUI's `app` state / `ui` rendering / event IO boundaries.

## Confirmed decisions

1. `/gitWorktree` appears only inside a closed template block. It binds that block's file mentions; it never switches the active workspace. Choosing another worktree writes/replaces a dedicated `{wt:<branch>}` line at the bottom of the block (the line above the end line) and keeps one blank line above it; choosing the main checkout instead removes the binding line and its blank separator (the main checkout is the default context), and the write shifts the cursor by edit position so it never lands on the following line. That line is structural metadata and must not be deleted or locally edited. For a linked-worktree binding the `@` panel first asks which scan root to use (`main` = repository main checkout, `wt` = bound checkout), then lists that checkout's candidates under a `Files-[main]` / `Files-[wt]` title; binding the main checkout skips the question. The command UI has no clear option.
2. Store the full Git branch name, not a checkout path or shortened display name. Omit detached and bare worktrees. A missing or ambiguous branch keeps the marker intact and makes `@` candidates unavailable in that block, with an explicit error.
3. Custom commands have User and Project scopes. Project commands are shared by all checkouts of one repository; non-Git workspaces use their own workspace root. Project overrides User on name collision; built-in names are reserved.
4. Custom commands support Markdown insertion (global or template-only) and complete `&&&` template-block insertion (block exterior only). Do not add the reference app's other command types.
5. The prompt's bottom-left icon opens an English HTML settings UI in the default browser, served on an ephemeral `127.0.0.1` port while the TUI runs.
6. Command rows show `[Built-in]`, `[User]`, `[Project]`, or `[Git]` on the right.
7. Persist settings in a user-level `settings.json` and custom commands as separate `.md` files. Use `.lx-t` for both User and Project roots; development User root is `.lx-t-dev`, matching the existing database split.

## Existing behavior and affected paths

- `src/app/markdown/slash.rs` has a fixed `SlashCommandId` list, filters only outside templates, and builds built-in template text. `Prompt.slash_confirm` in `src/app/prompt/panel.rs` replaces a whole slash line with that text.
- `src/app/markdown/template.rs` already parses `{wt:...}` on template end lines. `sync_mention_root` in `src/app/update.rs` currently uses explicit prompt root or prompt-owner cwd; it does not inspect the current template block.
- `src/git/` already queries `git worktree list --porcelain`; `WorkspaceGit` provides `repo_root` and `checkout_path`. The existing worktree-open dialog owns its list only while open, so the slash chooser needs its own request/result state or a shared repository cache.
- `src/layout.rs` owns prompt-toolbar button rectangles; `src/ui/prompt.rs` draws the bar. The settings icon sits at the left end of the sidebar's bottom border (mirroring the sidebar collapse button on the right); it is hidden when the sidebar is collapsed or the settings service is unavailable. Hit testing must share the layout geometry used by rendering.
- `src/db/mod.rs` already stores SQLite at `~/.lx-t/db/lx.db` (`~/.lx-t-dev/db/lx.db` in development). `src/config/mod.rs` currently has defaults and no settings file loader.

## Data and storage

### Directories

| Scope | Path |
| --- | --- |
| User, release | `~/.lx-t/settings.json`, `~/.lx-t/command/agentMD/<name>.md`, `~/.lx-t/command/agentBlock/<name>.md` |
| User, development | Same layout under `~/.lx-t-dev/` |
| Project, Git | `<main-checkout>/.lx-t/command/{agentMD,agentBlock}/<name>.md` |
| Project, non-Git | `<workspace-root>/.lx-t/command/{agentMD,agentBlock}/<name>.md` |

Use the repository main checkout's `repo_root`, not the linked checkout or shell's changing cwd. The settings path shares the existing database's data root without moving or rewriting SQLite. A missing settings file means defaults; unknown JSON keys are ignored and invalid known values return a visible save error. Write files atomically with a temporary file in the same directory and rename. Deleting or renaming a command must never silently overwrite a different command.

### Command file format

Use UTF-8 Markdown with a small frontmatter subset aligned with the reference: `description`, optional `argument-hint`, plus `scope: global|template` for `agentMD`, or `block: template` and optional `title` for `agentBlock`. The filename is the command name. Parse frontmatter with a structured parser; reject malformed metadata instead of executing it as body. Names use ASCII letters, digits, `_`, `-`; built-in aliases and names are reserved. The body is the inserted Markdown. For block commands, normalize a pasted complete block to one start/end pair and place the cursor inside `{title: }`.

Load User and Project commands for the prompt's displayed workspace, then overlay by name. On save/delete, send a refresh event to the TUI so the command panel updates without restarting. On workspace/prompt-owner change, reload the corresponding Project scope. Surface malformed files as settings errors while keeping valid commands available.

### `wt` resolution

The dedicated `{wt:<branch>}` line at the bottom of the block (the line above the end line) is the source of truth; no duplicate binding state lives in the prompt. At cursor movement and edit settlement, locate the enclosing closed template range and parse that binding line. If it is `{wt:<full-branch>}`, resolve that branch against a fresh worktree list for the prompt owner's `repo_root`. Exactly one existing checkout is required. Its checkout path and the repository's main checkout both become `@` scan roots for that block (the bound checkout first; identical relative paths keep the bound entry), ahead of the prompt's explicit root. Outside a bound block, keep today's explicit-root/cwd behavior. An unresolved marker sets a visible error state and disables only that block's `@` candidates; it never falls back to another root. Changing root invalidates the existing mention cache/generation so stale asynchronous results cannot appear. A non-Git prompt owner offers no `/gitWorktree` candidates. Structural blank rows render as block-border lines (`├─…─┤` in the block's status color) and carry no document text.

Selecting `/gitWorktree` opens a second-level worktree chooser. Show main checkout and linked entries with branches; show the currently bound entry. Confirm replaces the existing binding line or inserts one above the end line, keeps exactly one blank line above it, and clears the `/gitWorkTree` command line in place (the line itself is kept, mirroring the reference app's `/gitn`), all as one undo step. When the command line already occupies the binding position, the binding replaces that line's content. Selecting the same branch is harmless. If the list has no eligible entry, show an empty/error state without editing the block. Worktree queries stay in `src/git/` and run off the TUI event loop.

## UI and service

- Put a single-column settings icon at the far left of the sidebar's bottom border in expanded mode. Hide it when the sidebar is collapsed or too narrow, or when the settings service is unavailable. Use an icon supported by the terminal font and an English hover tooltip. A click asks the event layer to open the browser; `ui/` performs no IO.
- Reuse the existing command-panel widget. Add an optional right-aligned tag field (rendered once in brackets; the data layer passes plain labels) and reserve its width during layout so long names truncate before the tag. Keep rows and hit targets stable. Show built-in templates outside blocks, `/gitWorktree` and template-scoped Markdown inside closed blocks, and globally scoped Markdown in both locations. Worktree-related rows (`/gitWorkTree` and chooser entries) are tagged `[wt]`.
- Serve a single self-contained HTML page (minimal CSS and JavaScript) with a left navigation column and a right content area. Tabs: `General` and `Commands`; the latter has User/Project scope, Markdown/Template block type, list/create/edit/delete, name/description/scope fields, and a plain Markdown editor with preview. Display save errors and unsaved changes. All page text is English.
- Bind only `127.0.0.1:0`; use a per-run unguessable capability token and same-origin checks for writes. Restrict methods, request size and command filename/path handling. The service reads/writes through a dedicated persistence/settings layer and not through `ui/`; TUI state changes enter through event messages. Browser launch is in `src/platform/` behind OS-specific cfg. Stop the listener when the TUI exits, including normal shutdown and recovery paths.
- Persist current `Config` fields in `settings.json`, using defaults for missing fields and validating width relationships. The General tab edits the meaningful layout settings; internal safety bounds remain validated. Apply settings through app/event update after successful save, not inside rendering.

## Validation and acceptance

1. The slash chooser has the correct scope and tags, handles keyboard/mouse selection, and keeps tag width stable at narrow prompt sizes.
2. Binding-line insert/replace is one undo step; the line is protected from deletion/local edits; the command line is cleared in place and never removed; the active workspace does not change. Detached worktrees never appear.
3. Bound `@` scans the selected checkout, invalidates stale results on binding/cursor changes, and reports missing or ambiguous branches without fallback.
4. User/Project command precedence, reserved names, `.md` parsing, malformed files, atomic save/delete/rename, and live reload work across workspace switches and app restart.
5. Local Web UI opens from the icon, edits and previews Markdown, saves to `.lx-t`, rejects unauthorized/oversized/path-traversal requests, and closes with the TUI.
6. Run focused tests for changed app, git, settings, service, UI layout/panel, and event paths. Then run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` per project standards. User performs `cargo run` visual and browser interaction checks.

## Implementation gate

This document and `task.md` are for review. No production code or Git state changes occur until the user explicitly approves the design and task list.
