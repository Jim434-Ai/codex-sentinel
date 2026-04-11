# Specialist Bootstrap Pack

This pack is meant to be copied into a fresh specialist workspace before the
first run.

If you want a quick standard scaffold, prefer `codex specialist init`. Use this
bootstrap pack when you want Codex to interview the user and tailor the files
for a more custom setup.

## Purpose

The bootstrap pack gives Codex a repeatable way to:

- interview the user before writing config
- create `.codex/workspace.toml`
- create `.codex/machine.local.toml`
- create `.codex/task-contract.toml`
- create `AGENTS.md`
- create an issue-specific `.codex/issue.md`
- optionally create `.codex/soul.md`

This is intentionally a two-step flow:

1. start with a normal Codex session
2. use `codex exec --specialist` only after the bootstrap files exist

## Files To Copy Into The New Workspace

Copy these files into the target workspace root:

- `BOOTSTRAP_SPECIALIST.md`
- `WORKSPACE_LAYOUT.md`
- `SOURCE_ROOTS.md`
- `ISSUE_INTAKE.md`

Optional reference files:

- `workspace.template.toml`
- `machine.local.example.toml`

## Suggested First Prompt

Use a normal interactive Codex session in the new workspace and say:

```text
Read BOOTSTRAP_SPECIALIST.md and initialize this specialist workspace. Ask the minimum setup questions you need before creating any files.
```

## Expected Output Files

After the interview, the agent should create:

- `.codex/workspace.toml`
- `.codex/machine.local.toml`
- `.codex/task-contract.toml`
- `AGENTS.md`
- `.codex/issue.md`
- `.codex/soul.md` only if requested

## Recommended Validation

After bootstrap:

```bash
codex specialist validate-workspace
codex specialist show-context
```

Then the specialist run:

```bash
codex exec --specialist "Continue from the issue brief and pinned context."
```
