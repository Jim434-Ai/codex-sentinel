# Manager Workspaces

`codex manager` is the portable control surface for supervising multiple Codex
specialist agents from a manager workspace.

The first implementation intentionally preserves the existing pilot operating
model:

- a manager workspace owns the registry and authority documents;
- specialists are listed in `agents.tsv`;
- tmux remains the compatibility and observer layer;
- manager commands are deterministic lifecycle/status tools, not a replacement
  for the model-backed manager agent loop;
- `codex specialist worker` remains the lower-level specialist process protocol
  for later event-driven control.

## Files

```text
manager-workspace/
  AGENTS.md
  agents.tsv
  stage_two_authority.md
  manager_status.md
  events.log.md
  pings/
  .codex-manager/
    manager.toml
```

`agents.tsv` uses this column order:

```tsv
agent_id	matter_id	name	workspace	session	role	status	current_objective
```

The `workspace` field is the specialist workspace path, either absolute or
relative to the manager workspace. The `session` field is the tmux session name.
Agents with `closed`, `paused`, or `user-control` status are not started by
`start-active`.

## Commands

Validate or inspect a manager workspace:

```bash
codex manager validate
codex manager list
codex manager pings
```

Manage tmux-backed specialist lifecycle:

```bash
codex manager sessions
codex manager start agent-003
codex manager start-active
codex manager check agent-003 --lines 120
codex manager cycle --lines 80
codex manager stop agent-003
codex manager restart agent-003
```

Run from inside a manager workspace, or pass an explicit workspace:

```bash
codex manager --manager-workspace /path/to/manager-workspace validate
```

Override runtime settings with CLI flags or environment variables:

```bash
codex manager --tmux-socket codex-clean start-active
SPECIALIST_CODEX_BIN=/path/to/codex codex manager start-active
```

## Bootstrap

Copy the template files from `docs/manager-bootstrap/` into a new manager
workspace, edit `agents.tsv`, and update `.codex-manager/manager.toml` for the
local machine.

The manager authority model should stay in the workspace files. The CLI should
load and enforce the portable registry/lifecycle mechanics without hardcoding a
specific manager folder.
