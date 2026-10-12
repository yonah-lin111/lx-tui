# Prompt worktree and custom commands - tasks

- Status: implemented on `feat/prompt-worktree-custom-commands`; pending user merge review.
- Notes: 选择器绑定与 `/gitWorkTree` 均需光标位于模板块内（触发不跨块）；设置页仅编辑 User 作用域，Project 只读。
- Rule: keep `app/` pure, `ui/` read-only, Git execution in `git/`, OS launch in `platform/`, and blocking file/HTTP work off the event loop.

## 1. Models and storage

- [x] Extend configuration loading/saving for `~/.lx-t/settings.json` (development: `~/.lx-t-dev/settings.json`), defaults, validation, and atomic writes; keep the existing database path intact.
- [x] Add User/Project `.lx-t/command/{agentMD,agentBlock}` Markdown file loading and CRUD with structured frontmatter parsing, safe names, collision rules, and atomic writes.
- [x] Add focused storage tests for scope roots, linked worktrees, precedence, reserved names, malformed metadata, rename/delete, and restart persistence.

## 2. Worktree binding and mentions

- [ ] Add template-aware `/gitWorktree` candidates and a second-level chooser using background `git/` worktree results; exclude detached/bare entries.
- [x] Insert or replace `{wt:<full-branch>}` on the block end line as one undoable edit; preserve `{id:...}` and status and do not alter the active workspace.
- [x] Resolve bound branch to exactly one checkout for block-local `@` scans; invalidate stale scan results and surface unavailable/ambiguous bindings without fallback.
- [x] Add focused app/event tests for block-only visibility, selection, replacement, undo, deleted worktree, duplicate branch, root precedence, and async result generation.

## 3. Custom slash commands

- [x] Replace the fixed slash item representation with a compact built-in/custom item model; preserve current fuzzy matching and selection behavior.
- [x] Insert Markdown bodies in global or template-only scope and full template blocks outside existing blocks, with a single undo step and correct cursor placement.
- [x] Reload effective User/Project commands after settings writes and prompt-owner changes; add focused filtering, precedence, and insertion tests.

## 4. Prompt UI

- [x] Add the bottom-left settings icon geometry, rendering, hit testing, and English hover tooltip; maintain narrow/collapsed layouts.
- [x] Add right-aligned bracket tags to slash rows and account for their width in truncation/layout and mouse hit tests.
- [x] Add focused layout and rendering tests for icon, command tags, and minimum widths.

## 5. Local settings Web UI

- [x] Add a loopback-only, ephemeral-port HTTP service with per-run token, origin/method/body limits, clean shutdown, and event messages to update TUI state.
- [x] Add OS-gated default-browser launch in `src/platform/` and report open/save failures in TUI or page feedback.
- [x] Build a self-contained English HTML settings page: General and Commands tabs, User/Project and Markdown/Template block controls, list/create/edit/delete, Markdown source and preview, save/error/dirty states.
- [x] Test endpoints and storage behavior without a browser; verify token, origin, traversal, size limit, and shutdown paths.

## 6. Review and validation

- [x] Review changed paths against `docs/standards/` and preserve existing unrelated worktree edits.
- [x] Run focused module/integration tests, `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
- [ ] Ask the user to run `cargo run` (pending) and check TUI appearance and browser behavior interactively.
