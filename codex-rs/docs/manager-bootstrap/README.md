# Manager Workspace Bootstrap

This directory is a portable starting point for a manager workspace. Copy these
files into a new folder when creating a manager that supervises several Codex
specialist agents.

```text
manager-workspace/
  AGENTS.md
  agents.tsv
  manager_agent_protocol.md
  stage_two_authority.md
  manager_status.md
  manager_issues_for_user.md
  events.log.md
  pings/
  .codex-manager/
    manager.toml
```

After copying, edit:

- `agents.tsv` with each specialist workspace and tmux session name. Specialist
  workspace paths may be absolute or relative to the manager workspace.
- `.codex-manager/manager.toml` with local tmux and Codex binary settings.
- `AGENTS.md` with manager-specific authority and reporting rules.

Use from the manager workspace:

```bash
codex manager validate
codex manager start-active
codex manager cycle --lines 80
codex manager watch --interval-seconds 30
codex manager daemon --restart-missing
```

Use direct prompt delivery for tmux-backed specialists:

```bash
codex manager prompt agent-003 --delivery auto Continue the review.
```

Use the structured worker protocol path for one-shot status or prompt turns:

```bash
codex manager worker-status agent-003
codex manager worker-prompt agent-003 Continue the review.
codex manager worker-daemon --interval-seconds 30
codex manager worker-enqueue agent-003 Continue the review.
```

`worker-daemon` is event-first. It requests status at startup and then only
sends heartbeat status check-ins after the worker has been quiet; the default
heartbeat is 15 minutes. When attached to a terminal, it accepts interactive
commands: `help`, `agents`, `status [agent-id|all]`,
`prompt <agent-id> <message>`, and `quit`.

This bootstrap is intentionally separate from `docs/specialist-bootstrap/` so
manager and specialist templates can evolve independently.
