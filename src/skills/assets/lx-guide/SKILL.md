---
description: "Use for lx self-knowledge, features, built-in tools (read, edit, write, search, exec_command, write_stdin, web_search, ask, update_plan, curr_time, and MCP resource tools), tool surfaces, slash commands (/), @ mentions (@[refer-*]), subagent roles (explorer, executor, monitor, and custom .lx/agents/*.json roles), skills, MCP servers, settings, keybindings, and troubleshooting—including when the user asks about 'you', 'lx', 'this agent', or what a reference like @[refer-subagent](...) means."
---

# lx Guide

`lx` is a terminal coding agent. This skill is the built-in manual for `lx` itself: its tools, features, `@` mentions, `/` slash commands, subagent roles and configuration, skills, MCP servers, settings, keybindings, and troubleshooting.

## Choose one primary route

Use the first matching route, and read its reference only when the requested task needs that specialized workflow:

| Topic | Reference |
| --- | --- |
| Built-in tools and tool surfaces (`read`, `edit`, `write`, `search`, `exec_command`, `write_stdin`, `web_search`, `ask`, `update_plan`, `curr_time`), parameters and limits, patch grammar, process/background behavior, MCP tools, and MCP resource tools | `references/tools.md` |
| Subagents, built-in roles (`explorer`, `executor`, `monitor`), custom roles (`.lx/agents/*.json`, `.agents/roles/*.json`, `subagents.json`), `@[refer-subagent](...)`, and the 8 subagent control tools | `references/subagents.md` |
| Slash commands (`/`), `@` mention tabs (`Files`, `refDir`, `Skills`, `MCP`, `Subagents`), `@[refer-*]` reference tokens, the Prompt panel, settings, and keybindings | `references/commands-and-mentions.md` |

Read at most one primary reference unless the user's question genuinely spans multiple routes. Do not open every reference file preemptively.

## How to answer questions about lx

When the user asks about `lx`, this agent, or any built-in feature (for example, what an `@[refer-subagent](...)` reference means), read the matching reference file in this skill directory with the `read` tool. Resolve paths relative to the directory containing this `SKILL.md`.

## Critical boundary

Do NOT search or read the user's current workspace source tree to guess how `lx` works. Answer directly from the `<subagent_reference>` blocks injected into the turn context and from the reference files in this `lx-guide` skill. Only inspect the workspace when the user asks about their own project code rather than about `lx` itself.
