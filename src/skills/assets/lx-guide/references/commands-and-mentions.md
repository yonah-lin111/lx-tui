# Commands and mentions

This reference covers the `@` mention panel, the `/` slash command panel, the right-hand Prompt panel, settings, and keybindings.

## `@` mention panel

Type `@` at the start of a line, after whitespace, or after `[` to open the mention panel in the `lx` composer. The panel has five tabs, in this order:

| Tab | Contents | Inserted reference |
| --- | --- | --- |
| `Files` | Workspace files and folders, respecting `.gitignore`. | `@path` plus a trailing space; the explicit forms `@[refer-file](path)`, `@[refer-folder](path)`, and `@[refer-image](path)` are also accepted |
| `refDir` | Referenced external folders added from the Prompt panel or `lx-tui prompt ref add`. | `@[refer-folder](path)` |
| `Skills` | Available project, user, and built-in system skills. | `@[refer-skill](<SKILL.md absolute path>)` |
| `MCP` | Configured MCP servers. | `@[refer-mcp](<server_name>)` |
| `Subagents` | Active session subagents, built-in roles, and custom roles. | `@[refer-subagent](<role>)` or `@[refer-subagent](<nickname_or_id>:<role>)` |

Navigation and selection:

- `←` / `→` cycle the tabs.
- `↑` / `↓` move the highlight; the mouse wheel scrolls the candidate viewport.
- `Enter`, `Tab`, or a left click confirms the highlighted entry.
- `Esc` closes the panel.
- In the `Files` tab, `Shift+Enter` opens the highlighted directory and the editor undo key (`Ctrl+Z` / `Cmd+Z`) goes back one level.
- A trailing space is inserted after the reference. Pressing `Backspace` immediately after that space deletes the whole `@` token or `@[refer-*](...)` block in one step.

What each reference does when the message is sent:

- File, folder, and image tokens become message attachments. Images are loaded and resized as image input.
- `@[refer-skill](...)` injects the referenced `SKILL.md` body into the current turn (truncated to 8,000 bytes).
- `@[refer-mcp](...)` marks an MCP server by name so the main agent knows which server the question targets. It does not enable a disabled server; MCP tools come from the pane's active MCP configuration, and the reference is carried as plain text.
- `@[refer-subagent](...)` injects a `<subagent_reference>` block describing the role or instance; see `references/subagents.md`.

The right-hand Prompt panel keeps the original four tabs (`Files`, `refDir`, `Skills`, `MCP`); it does not show `Subagents`. The `AiWeb` entry shows only the `Files` tab: sending a folder reference is rejected as folder-unsupported, and `refer-skill`, `refer-mcp`, or `refer-subagent` references are rejected as local-reference-unsupported.

## Slash commands (`/`)

Type `/` at the start of a line in the composer to open the command panel. Built-in commands are matched by prefix or fuzzy subsequence while the current line is a single command token (no spaces). Commands normally run from that single-token line; `/rename` and its alias `/title` are the exception and accept a space-separated title argument on the same line (`/rename New session title`). Any other line containing spaces is sent as ordinary text.

| Command | Aliases | Description |
| --- | --- | --- |
| `/new` | `/clear` | Start a fresh blank session in the current pane. |
| `/resume` | `/session` | Pick a previous session for this workspace and load it. |
| `/fork` | — | Fork the current session from before a chosen user message. |
| `/rewind` | `/undo` | Undo the latest turn, reverting files and conversation. |
| `/compact` | `/summarize` | Compress completed turns into a summary checkpoint. |
| `/model` | — | Choose the model and reasoning level for the next turn. |
| `/connect` | — | Connect a preset provider with an API key or OAuth. |
| `/subagents` | `/agents` | Open the background subagents panel. |
| `/mcp` | `/mcps` | Enable, disable, or manage MCP servers for the current pane. |
| `/rename` | `/title` | Rename the current session; `/rename <title>` (or `/title <title>`) takes the new title on the same line. |

Notes:

- There is no built-in `/skills` or `/settings` slash command in this build. Use `lx-tui skill list` (see `lx-tui skill --help` for the full command set) and `lx-tui settings show` (or `lx-tui settings open` for the browser UI) for settings. Those names can still exist as user-defined custom slash commands.
- In `lx-tui exec`, point at a skill in the prompt with `$<skill-name>` (for example, `Use $shell-style and refactor ...`) to inject that skill's `SKILL.md` body into the run.
- Custom slash commands (`agentMD` and `agentBlock`) are managed with `lx-tui command` or the browser settings page and appear in the same `/` panel with a `User` or `Project` source tag.
- The Prompt editor uses the same `/` panel for template and supplemental block commands.

## Prompt panel

The right-hand container is a global, workspace-scoped prompt editor shared by all tabs and workspaces. It has `Prompt` and `Todo` tabs on its top border. The Prompt panel stores prompts in folders with multiple pages, supports an editing view, a tree view, and a page-list view, and has a `refDir` button for adding referenced external folders used by the `refDir` mention tab.

Common Prompt panel keys:

- `Ctrl/Cmd+S` saves the current entry (entries also auto-save after a short idle period).
- `Ctrl/Cmd+L` switches between the tree view and the editing view.
- `Ctrl/Cmd+P` switches between the page list and the editing view.
- `Ctrl/Cmd+←/→` switches pages; `Ctrl/Cmd+Shift+→` creates a page.
- `Ctrl/Cmd+R` toggles the recent-pages drawer.
- `Ctrl/Cmd+Shift+C` copies the body of the `&&&` template block or top-level `+++` supplemental block under the cursor.

## Settings

`lx-tui settings` requires a subcommand; it manages layout settings and the browser settings UI:

- `lx-tui settings show` prints the current layout settings and config paths.
- `lx-tui settings set ...` updates `settings.json` (sidebar width and minimum window/pane sizes).
- `lx-tui settings reset` restores defaults.
- `lx-tui settings open [--no-open]` starts the local browser settings server on `127.0.0.1` until `Ctrl+C`.

The browser web console covers a Home landing page (LX pixel logo plus Settings / Usage entry cards) and the Settings / Usage views: providers and models, connect/OAuth, MCP servers, skills, custom slash commands, and subagent roles, plus token/cost/request analytics. In the TUI, the sidebar's `[WEB]` button opens the unified console at Home; the top bar switches between Home, Settings, and Usage. The `Subagents`, `Skills`, and `MCP` sections write user-scoped config under `<data_root>/` (`config/subagents.json`, `config/skills.json`, and the MCP config).

Most settings changes are hot-reloaded: the settings service emits a change event and the TUI re-scans skills, MCP servers, commands, models, or subagent roles without a restart.

## Skill and subagent storage

Skills are discovered from five roots; earlier roots win when two skills share a name:

| Priority | Source | Location |
| --- | --- | --- |
| 1 | Project lx skill | `<data_root>/skills/projects/<scope>/` |
| 2 | Project `.agents` skill | `<workspace>/.agents/skills/` |
| 3 | User lx skill | `<data_root>/skills/` |
| 4 | User `.agents` skill | `~/.agents/skills/` |
| 5 | Built-in system skill | `<data_root>/skills/.system/lx-guide/` |

Only lx-owned skills are editable from the settings page and CLI. `.agents` skills are read-only and disabled by default; enable one explicitly from the Skills page or with `lx-tui skill enable <name>`. The built-in `lx-guide` system skill is installed automatically and is also read-only.

Project subagent roles live in `<workspace>/.lx/agents/*.json` and `<workspace>/.agents/roles/*.json`. The browser `/settings` Subagents page edits the user-level `<data_root>/config/subagents.json` (`subagents.roles`).

## Keybindings

The TUI does not expose a user-editable keymap; the fixed bindings are:

Composer (`lx` pane):

- `Enter` sends the message; `Shift+Enter` inserts a newline; `Ctrl/Cmd+Enter` inserts a newline below.
- `@` opens mentions; `/` opens commands; `Ctrl/Cmd+Z` / `Ctrl/Cmd+Y` undo and redo; `Ctrl/Cmd+C` clears the draft (or copies when a selection exists); `Ctrl/Cmd+X` cuts; `Ctrl/Cmd+Shift+A` selects all.
- `Ctrl+U` / `Ctrl+K` delete to line start / end; `Ctrl+W` deletes a word backward; `Ctrl+A` / `Ctrl+E` go to line start / end.
- `↑` / `↓` browse prompt history when no panel is open; `↓` on an empty draft opens the background task page.
- `Ctrl+O` opens the background task panel focused on the current tool/process card; `Alt+↑` / `Alt+↓` select adjacent tool cards.
- `Tab` queues the current draft as input for the running turn.
- `Ctrl+T` toggles the latest thinking block.
- `Esc` closes the active panel or overlay without sending; with no panel open it stops the current turn (and exits an idle shell).

Panels and overlays:

- `Esc` closes the active overlay; `Up` / `Down` move the selection; `Enter` confirms.
- In the Prompt panel, `Tab` / `Shift+Tab` indent or outdent, and `Shift+Tab` cycles a closed `&&&` block status when the cursor is inside one.

Global:

- `Ctrl+Q` quits the application.
- `Ctrl+C` copies a selection when one exists; otherwise it clears the composer draft.

These bindings are the behavior of the current build; there is no keybinding configuration file.
