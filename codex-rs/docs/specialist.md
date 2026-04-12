# Specialist Workspaces

`Codex Sentinel v0.0.2` added a thin specialist workflow layer on top of the
existing CLI. `Codex Sentinel v0.0.3` starts the manager-owned lifecycle path.
The current specialist workflow now covers:

- workspace manifest resolution
- machine-local root mapping
- task-contract loading for objective/phase/deliverable control
- pinned context-set inspection
- checkpoint status inspection
- specialist-aware interactive `codex` runs
- specialist-aware `codex exec` runs
- specialist checkpoint emission and resume state across interactive and exec runs
- JSONL specialist worker protocol for manager-owned lifecycle control

## Files

The specialist workflow expects these files under the workspace:

- committed manifest: `.codex/workspace.toml`
- local-only mapping: `.codex/machine.local.toml`
- task contract: `.codex/task-contract.toml`

The machine profile should stay out of Git because it resolves logical root ids
to machine-specific absolute paths.

The task contract is optional but strongly recommended. It gives the runtime a
structured statement of:

- `objective`
- `current_phase`
- `current_deliverable`
- `stop_conditions`
- `blocked_reasons`

## Workspace Manifest

Example:

```toml
workspace_id = "market"
issue_id = "market"
default_context_set = "core"

[roles.source]
root = "source_root"
path = "Statements"
access = "read-only"

[roles.analysis]
root = "analysis_root"
path = "Workproduct"
access = "read-write"

[context_sets.core]
files = [
  { role = "analysis", path = "00A_FRAME.md" },
  { role = "analysis", path = "06_Next_Actions.md" },
]
```

Fields:

- `workspace_id`: stable workspace identifier
- `issue_id`: optional issue/matter identifier; defaults to `workspace_id`
- `default_context_set`: optional default context set name
- `roles.<name>.root`: logical root id resolved by `machine.local.toml`
- `roles.<name>.path`: optional subpath under that logical root
- `roles.<name>.access`: `read-only` or `read-write`
- `context_sets.<name>.files`: ordered file list for that context set

## Machine Profile

Example:

```toml
[roots]
source_root = "/Users/jamescharles/Desktop/Market"
analysis_root = "/Users/jamescharles/Desktop/Market"
```

Root paths may differ across machines as long as the logical ids stay the same.

## Commands

Initialize a specialist workspace scaffold in the current directory:

```bash
codex specialist init
codex specialist init --yes --workspace-id market --issue-id market-2026
```

Validate the workspace:

```bash
codex specialist validate-workspace
```

Show the resolved files for the default or named context set:

```bash
codex specialist show-context
codex specialist show-context --context-set core
```

Read the latest checkpoint state:

```bash
codex specialist status
codex specialist status --json
```

Run `codex exec` in specialist mode:

```bash
codex exec --specialist "Draft the first pass of the memo"
codex exec --specialist --context-set core "Continue the analysis"
```

Run the interactive TUI in specialist mode:

```bash
codex --specialist
codex --specialist --context-set core
```

Run the JSONL specialist worker loop for manager-owned lifecycle control:

```bash
codex specialist worker
codex specialist worker --context-set core
codex specialist worker --dry-run
```

`codex specialist worker` is the first `Codex Sentinel v0.0.3` worker-mode
slice. It is designed for a manager daemon or other local supervisor, not direct
human prompting. The worker reads JSONL commands from stdin and writes JSONL
events to stdout. Terminal multiplexers such as tmux can still observe logs or
transcripts, but they should not be the lifecycle control plane.

Supported worker commands:

```jsonl
{"type":"status","id":"status-1"}
{"type":"prompt","id":"turn-1","prompt":"Continue the next bounded specialist task."}
{"type":"shutdown","id":"shutdown-1"}
```

Representative worker events:

```jsonl
{"type":"ready","workspace_id":"matter","issue_id":"matter","worker_pid":12345}
{"type":"status","command_id":"status-1","workspace_id":"matter","issue_id":"matter","checkpoint":null}
{"type":"turn_started","command_id":"turn-1"}
{"type":"turn_completed","command_id":"turn-1","exit_code":0,"final_message":"..."}
{"type":"needs_direction","command_id":"turn-1","reason":"prompt turn completed; manager should inspect the latest checkpoint and decide the next action"}
{"type":"exited","command_id":"shutdown-1","reason":"shutdown command received"}
```

Prompt commands currently run `codex exec --specialist --json` as a bounded
child process and then emit `turn_completed` plus `needs_direction`. This is an
MVP control surface for manager-daemon integration; later work should replace
the child-process bridge with a direct worker runtime that streams structured
turn events as they happen.

When `--specialist` is enabled:

- the specialist workspace root becomes the effective `cwd`
- read-only roles are added as readable source roots
- read-write roles outside the workspace root become additional writable roots
- the task contract and latest checkpoint are injected into specialist prompts
- the selected context set is injected into the opening prompt
- interactive TUI runs apply the specialist sandbox and prompt wrapping for
  each user turn and now write the latest checkpoint after terminal turns
- `codex exec --specialist` writes the latest checkpoint under `CODEX_HOME`
  after the run completes
- both interactive and exec specialist runs load the latest checkpoint back
  into the next prompt for resume continuity

Multiple external source folders should be modeled as separate roles with
different logical roots in `machine.local.toml`.

Interactive specialist mode is currently supported only for local TUI sessions.
Remote app-server sessions still reject `--specialist`.

## Checkpoints

Checkpoint state is stored under `CODEX_HOME`, not under the project-local
`.codex/` directory:

```text
$CODEX_HOME/specialist/checkpoints/<workspace_id>/<issue_id>/latest.json
$CODEX_HOME/specialist/checkpoints/<workspace_id>/<issue_id>/latest.md
```

This keeps mutable specialist state writable while preserving the existing
project-local `.codex` protections.

## Task Contract

Example:

```toml
objective = "Draft the first diligence memo"
current_phase = "fact review"
current_deliverable = "deliverables/first-pass.md"
stop_conditions = [
  "Update the current deliverable",
  "List the remaining open questions",
]
blocked_reasons = []
```

This file is designed to keep specialist runs bounded. The prompt wrapper uses
it to reinforce what the agent is trying to finish before it stops.

## Bootstrap Pack

For standard setups, prefer `codex specialist init`.

If you want Codex to interview the user and generate or customize specialist
files for a new workspace, use the bootstrap pack in:

```text
docs/specialist-bootstrap/
```

Recommended flow:

1. Copy the bootstrap markdown files into the new workspace root.
2. Start a normal interactive Codex session in that workspace.
3. Prompt Codex to read `BOOTSTRAP_SPECIALIST.md` and ask setup questions.
4. Let Codex create `.codex/workspace.toml`, `.codex/machine.local.toml`,
   `.codex/task-contract.toml`, `AGENTS.md`, and `.codex/issue.md`.
5. Then switch to `codex --specialist` for interactive work or
   `codex exec --specialist` for a one-shot run.
