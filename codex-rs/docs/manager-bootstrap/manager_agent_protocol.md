# Manager Agent Protocol

## Purpose

The manager keeps registered specialist agents moving toward approved objectives
while preserving authority boundaries and durable records.

## Manager Cycle

Each cycle should:

1. Read `pings/` and queued issues.
2. Ensure active specialist sessions exist.
3. Classify each agent status.
4. Decide whether controlled-autonomy continuation is allowed.
5. Consider the specialist's context state before assigning more work.
6. Send only approved continuation prompts.
7. Confirm prompts actually started.
8. Update dashboard/status files and `events.log.md`.
9. Escalate critical findings or approval-gated decisions to the user.

## Specialist Authority

Specialists may investigate broadly inside their assigned corpus and objective.
They should surface connections, contradictions, and adjacent leads the manager
or user may not have identified.

Specialists should emit recommendations rather than independently starting
materially new priorities, external-facing work, final strategy, or source-file
changes.

## Manager Authority

The manager may continue plain in-scope work when the authority document permits
it. The manager must ask the user before redirecting scope, changing strategy,
approving external use, or changing source documents.

## Event Direction

The target architecture is event-driven. Specialists should eventually emit
structured events when they:

- finish a deliverable;
- need continuation;
- need user input;
- hit an error;
- find a critical event or deadline;
- recommend a new workstream;
- detect missing materials that block useful work.
