# Bootstrap Specialist Workspace

You are initializing a specialist workspace.

Your job is to interview the user, then create deterministic workspace files
for later `codex exec --specialist` runs.

## Operating Sequence

1. Read `WORKSPACE_LAYOUT.md`, `SOURCE_ROOTS.md`, and `ISSUE_INTAKE.md`.
2. Identify missing information and ask the user the minimum questions needed.
3. Do not create config files until the key answers are collected.
4. After the interview, create the required files.
5. Validate the workspace if the local Codex build supports `codex specialist`.
6. Summarize what was created and show the exact next run command.

## Required Questions

Ask about these topics if they are missing or still placeholders:

- workspace id
- issue id or matter id
- main deliverable
- writable workspace folders
- external source folders
- whether each source folder should be its own role
- default pinned context files
- operating rules for `AGENTS.md`
- whether a `.codex/soul.md` should exist
- if `soul.md` should exist, what role/persona it should capture

## Required Output Files

Create these files:

- `.codex/workspace.toml`
- `.codex/machine.local.toml`
- `AGENTS.md`
- `.codex/issue.md`

Create `.codex/soul.md` only if the user wants it.

## File Rules

### `.codex/workspace.toml`

- use separate roles for separate source roots
- mark external sources as `read-only`
- mark the workspace analysis area as `read-write`
- include a default context set
- include `AGENTS.md` and `.codex/issue.md` in the default context set
- include `.codex/soul.md` in the default context set only if it exists

### `.codex/machine.local.toml`

- use absolute paths
- map logical root ids to real machine paths
- keep machine-specific values out of the committed manifest

### `AGENTS.md`

This is the workspace-wide operating contract.

It should define:

- authoritative source folders
- writable output folders
- naming conventions
- citation expectations
- what must never be modified
- how to treat derived outputs vs originals

### `.codex/issue.md`

This is the issue-specific brief.

It should define:

- issue title
- objective
- deliverables
- constraints
- open questions
- success criteria
- next recommended run prompt

### `.codex/soul.md`

Create only if explicitly wanted.

If created, keep it short and stable. It should capture:

- role identity
- tone
- evidence discipline
- safety boundaries
- output expectations

## Constraints

- Prefer asking one compact batch of questions over many tiny interruptions.
- Do not guess absolute machine paths if the user has not provided them.
- Do not create a single source role that points to multiple unrelated source roots.
- Use one external source folder per logical root unless the user explicitly wants otherwise.
- If the user is unsure about `soul.md`, skip it and proceed with `AGENTS.md` plus `.codex/issue.md`.

## Finish Line

Before finishing:

- show which files were created
- show which roles were defined
- show the default context set name
- show the exact `codex exec --specialist ...` command to use next
