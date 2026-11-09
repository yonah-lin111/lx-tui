# Subagents

`lx` can delegate work to subagents: isolated child agent sessions with their own role, tool whitelist, model, and context. The main `lx` agent orchestrates them through the subagent control tools; the user can point at a role or a live instance from the `@` mention panel in the composer.

## `@[refer-subagent](...)`

The `Subagents` tab of the `@` mention panel inserts one of two reference forms:

- `@[refer-subagent](<role>)` — a built-in or custom role, for example `@[refer-subagent](explorer)`.
- `@[refer-subagent](<nickname_or_id>:<role>)` — a live subagent instance in the current session, for example `@[refer-subagent](Turing:explorer)`. Only instances in `running`, `idle`, or `blocked` state are listed.

The panel ordering is: active session instances first, then the built-in roles (`explorer`, `executor`, `monitor`), then custom roles (project-scoped first, then user-scoped). Filtering is case-insensitive over nickname, agent id, role, status, title, description, and category words.

On submission, `lx` stores the reference as plain text in the user message and injects an English `<subagent_reference>` block into the current turn's model context. The block is not persisted and does not create a new message.

- For a role reference, the block contains the full role specification: `role`, `base_role`, `scope` (`built-in`, `custom (project)`, or `custom (user)`), `description`, the effective `tools` whitelist, `capabilities` (`allow_web_search`, `allow_mcp`, `allow_skills`), `model` (`inherits parent model` or an explicit provider/model and whether it is locked), and the effective `system_prompt`.
- For a live instance reference, the block contains `agent_id`, `nickname`, `role`, `status`, `agent_path`, `title`, `task`, and `latest_report` when available.

The guidance in the block distinguishes two intents: if the user is asking what the role or instance is, the main agent answers directly from the injected specification without searching the workspace; if the user is delegating a task, the main agent calls `spawn_agent` for the role or `send_input` / `wait_agent` for the live instance.

Referencing a subagent does not bypass the main agent and does not send a private message to the child. Direct conversation with a child is done from the `/subagents` panel (`TaskPage::Agents`), where `Enter` opens an inline reply box.

## Built-in roles

`BaseRole` defines three permission tiers. Built-in role names are reserved and cannot be redefined by custom roles.

| Role | Purpose | Tools |
| --- | --- | --- |
| `explorer` | Read-only codebase and web investigator. | `read`, `search`, `web_search` (3) |
| `executor` | Bounded implementation worker. | `read`, `search`, `web_search`, `edit`, `write`, `exec_command`, `write_stdin` (7); plus the 8 subagent control tools when spawned below `maxDepth` (`ExecutorCoordinator`, 15 total) |
| `monitor` | Long-running command and process watcher. | `read`, `search`, `exec_command`, `write_stdin` (4) |

- `explorer` has zero write, process, or delegation permissions and is always a leaf node at any depth.
- `executor` has no `ask` and no `update_plan`. It only gains delegation tools when its depth is below `maxDepth`; at the depth limit it is a leaf.
- `monitor` cannot edit or write files and cannot delegate; it is always a leaf node.

`executor`-based roles are the only ones that can recursively coordinate subagents. The default maximum depth is `2` (main agent → subagent → grandchild) and the maximum concurrency across the whole root session tree is `4`.

## Custom roles and configuration

Project-scoped roles load only when the workspace is trusted:

1. `<workspace>/.lx/agents/*.json` — highest precedence among project role files.
2. `<workspace>/.agents/roles/*.json`.

User-scoped roles live in `<data_root>/config/subagents.json` under `subagents.roles`, and can be managed from the `/subagents` panel, the browser settings page (`lx-tui settings open`), or the `lx-tui subagent` CLI.

If a global user role and a project role share a name, the global role wins. Built-in names (`explorer`, `executor`, `monitor`) can never be overridden by a custom role. Project role discovery rejects symlinks, hidden files, files larger than 64 KiB, malformed JSON, unknown fields, and more than 32 project roles per workspace; invalid files are skipped with a warning and do not break the session.

Each file in `.lx/agents/` or `.agents/roles/` is one role object. `name` is optional and defaults to the file stem.

```json
{
  "name": "reviewer",
  "baseRole": "explorer",
  "description": "Reviews diffs for correctness and regressions.",
  "systemPrompt": "You review code changes. Report concrete defects with file and line references.",
  "model": {
    "provider": "openai",
    "model": "gpt-5",
    "variant": "high"
  },
  "lockModel": false,
  "allowWebSearch": true,
  "allowMcp": false,
  "allowSkills": true,
  "nicknameCandidates": ["Turing", "Hopper"]
}
```

Fields:

- `name` — must match `[a-z][a-z0-9_]{0,31}` and must not collide with a built-in or global custom role.
- `baseRole` — `"explorer"`, `"executor"`, or `"monitor"`; this fixes the permission tier and default system prompt.
- `description` — required, at most 500 characters; shown in `spawn_agent` tool descriptions and the `@` panel.
- `systemPrompt` — required, at most 16,000 characters.
- `model` — optional `{ "provider": "...", "model": "...", "variant": "..." }`; omitted means the subagent inherits the parent model.
- `lockModel` — when `true` and `model` is set, callers cannot override the model or reasoning level.
- `allowWebSearch`, `allowMcp`, `allowSkills` — optional booleans that can only tighten the `baseRole` permissions, never expand them. All default to `true`. A `monitor` never gets `web_search` regardless of `allowWebSearch`.
- `nicknameCandidates` — optional array of preferred nicknames; empty means the global scientist nickname pool.

`<data_root>/config/subagents.json` also contains the run-wide controls:

```json
{
  "subagents": {
    "maxConcurrent": 4,
    "maxDepth": 2,
    "defaultMaxTokens": null,
    "delegationMode": "auto",
    "builtins": {
      "explorer": {},
      "executor": {},
      "monitor": {}
    },
    "roles": []
  }
}
```

- `maxConcurrent` — `1..=12`, default `4`.
- `maxDepth` — `1..=3`, default `2`.
- `defaultMaxTokens` — optional per-turn token budget for a subagent; omitted means unlimited.
- `delegationMode` — `auto`, `proactive`, or `explicit`; controls how aggressively the main agent delegates.
- `builtins` — per-role overrides (`description`, `systemPrompt`, `model`, `lockModel`, and capability toggles) for `explorer`, `executor`, and `monitor`.
- `roles` — user-scoped custom roles, same schema as a project role file.

## Subagent control tools

The main agent has 8 control tools:

| Tool | Purpose |
| --- | --- |
| `spawn_agent` | Start a new subagent with a role, task, and optional model/effort overrides. |
| `send_input` | Send follow-up input to an existing subagent; queues when it is busy. |
| `interrupt_agent` | Stop the current turn of a subagent and its running descendants, keeping partial output and leaving it `idle`. |
| `resume_agent` | Resume a paused, closed, or completed subagent and consume queued input. |
| `wait_agent` | Block until a subagent finishes its current turn or times out. |
| `close_agent` | Close a subagent and its whole subtree without a partial report; history and queue are kept for a later resume. |
| `list_agents` | List the subagent tree of the root session, optionally filtered by `agent_path` prefix. |
| `cancel_agent_input` | Cancel queued (including `blocked`) input without touching a running turn or terminal state. |

## Statuses

- `running` — actively working on a turn.
- `idle` — alive and waiting for input.
- `blocked` — queue is blocked by a failed or lost recovery and needs explicit handling.
- `completed` — finished successfully.
- `failed` — finished with an error.
- `interrupted` — interrupted by the user or parent.
- `cancelled` — cancelled before completion.

`running`, `idle`, and `blocked` are the active states shown in the `@` panel and the `/subagents` active filter. The remaining states are terminal.
