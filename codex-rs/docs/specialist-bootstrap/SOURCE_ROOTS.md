# Source Roots Intake

Use one block per external source folder. Codex should ask follow-up questions
if any path, purpose, or role name is missing.

## Source Root 1

- Logical root id: `[example: statements_root]`
- Suggested role name: `[example: source_statements]`
- Absolute path: `[fill in]`
- Description: `[what lives here]`
- Access: `read-only`

## Source Root 2

- Logical root id: `[example: taxes_root]`
- Suggested role name: `[example: source_taxes]`
- Absolute path: `[fill in]`
- Description: `[what lives here]`
- Access: `read-only`

## Source Root 3

- Logical root id: `[optional]`
- Suggested role name: `[optional]`
- Absolute path: `[optional]`
- Description: `[optional]`
- Access: `read-only`

## Notes

- Do not combine unrelated source folders into one logical root unless the user asks.
- If a source folder is actually writable, call that out explicitly so Codex can create a separate `read-write` role instead of treating it as source-only.
