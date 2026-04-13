# Manager Workspaces

`codex manager` is the portable control surface for supervising multiple Codex
specialist agents from a manager workspace.

The manager command surface intentionally preserves the existing pilot
operating model while adding a foreground supervisor loop:

- a manager workspace owns the registry and authority documents;
- specialists are listed in `agents.tsv`;
- tmux remains the compatibility and observer layer;
- `watch` and `daemon` provide deterministic foreground supervision, not a
  model-backed manager agent loop;
- `worker-status` and `worker-prompt` provide the lower-level structured
  specialist protocol path for later event-driven control.

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
codex manager watch --interval-seconds 30
codex manager daemon --restart-missing
codex manager stop agent-003
codex manager restart agent-003
```

Prompt a tmux-backed specialist without taking over the whole terminal:

```bash
codex manager prompt agent-003 --delivery auto Continue the source review.
codex manager prompt agent-003 --delivery stage Draft this into the composer but do not interrupt.
codex manager prompt agent-003 --delivery submit Continue now.
```

`auto` submits when the specialist looks idle at the prompt and stages the text
without pressing Enter when the specialist looks busy.

Use the worker-backed path when you want a structured one-shot protocol instead
of a live TUI pane:

```bash
codex manager worker-status agent-003
codex manager worker-prompt agent-003 Continue the source review.
codex manager worker-prompt --dry-run agent-003 Test the worker protocol only.
```

Run active specialists as a persistent worker pool:

```bash
codex manager worker-daemon --interval-seconds 30
codex manager worker-enqueue agent-003 Continue the source review.
codex manager worker-daemon --dry-run --iterations 1
```

`worker-daemon` starts one `codex specialist worker` child per active agent,
polls status, restarts exited workers by default, and watches
`worker-prompts/<agent-id>/*.md` for queued prompts. `worker-enqueue` writes
those prompt files so a second terminal can feed a running worker daemon without
attaching to its process.

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
