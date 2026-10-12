# Prompt worktree and custom commands - tasks

- Status: awaiting design/task review; all implementation tasks are pending.
- Rule: keep `app/` pure, `ui/` read-only, Git execution in `git/`, OS launch in `platform/`, and blocking file/HTTP work off the event loop.

## 1. Models and storage

- [ ] Extend configuration loading/saving for `~/.lx-t/settings.json` (development: `~/.lx-t-dev/settings.json`), defaults, validation, and atomic writes; keep the existing database path intact.
- [ ] Add User/Project `.lx-t/command/{agentMD,agentBlock}` Markdown file loading and CRUD with structured frontmatter parsing, safe names, collision rules, and atomic writes.
- [ ] Add focused storage tests for scope roots, linked worktrees, precedence, reserved names, malformed metadata, rename/delete, and restart persistence.

## 2. Worktree binding and mentions

- [ ] Add template-aware `/gitWorktree` candidates and a second-level chooser using background `git/` worktree results; exclude detached/bare entries.
- [ ] Insert or replace `{wt:<full-branch>}` on the block end line as one undoable edit; preserve `{id:...}` and status and do not alter the active workspace.
- [ ] Resolve bound branch to exactly one checkout for block-local `@` scans; invalidate stale scan results and surface unavailable/ambiguous bindings without fallback.
- [ ] Add focused app/event tests for block-only visibility, selection, replacement, undo, deleted worktree, duplicate branch, root precedence, and async result generation.

## 3. Custom slash commands

- [ ] Replace the fixed slash item representation with a compact built-in/custom item model; preserve current fuzzy matching and selection behavior.
- [ ] Insert Markdown bodies in global or template-only scope and full template blocks outside existing blocks, with a single undo step and correct cursor placement.
- [ ] Reload effective User/Project commands after settings writes and prompt-owner changes; add focused filtering, precedence, and insertion tests.

## 4. Prompt UI

- [ ] Add the bottom-left settings icon geometry, rendering, hit testing, and English hover tooltip; maintain narrow/collapsed layouts.
- [ ] Add right-aligned bracket tags to slash rows and account for their width in truncation/layout and mouse hit tests.
- [ ] Add focused layout and rendering tests for icon, command tags, and minimum widths.

## 5. Local settings Web UI

- [ ] Add a loopback-only, ephemeral-port HTTP service with per-run token, origin/method/body limits, clean shutdown, and event messages to update TUI state.
- [ ] Add OS-gated default-browser launch in `src/platform/` and report open/save failures in TUI or page feedback.
- [ ] Build a self-contained English HTML settings page: General and Commands tabs, User/Project and Markdown/Template block controls, list/create/edit/delete, Markdown source and preview, save/error/dirty states.
- [ ] Test endpoints and storage behavior without a browser; verify token, origin, traversal, size limit, and shutdown paths.

## 6. Review and validation

- [ ] Review changed paths against `docs/standards/` and preserve existing unrelated worktree edits.
- [ ] Run focused module/integration tests, `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
- [ ] Ask the user to run `cargo run` and check TUI appearance and browser behavior interactively.
