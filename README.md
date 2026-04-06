# vima

A ticket tracker that lives in your repo. Tickets are Markdown files with YAML frontmatter stored in `.vima/tickets/`, designed to be read and written by AI agents and shell scripts as easily as by humans.

All output is **machine-readable JSON** by default. Every command exits 0 on success, non-zero on error. Errors go to stderr as JSON with structured `error`, `message`, and context fields.

## Install

### From source (requires Rust 1.70+)

```sh
cargo install --path .
```

### From GitHub releases

Download a prebuilt binary for your platform:

```sh
# Linux x86_64
curl -fsSL https://github.com/pheuberger/vima/releases/latest/download/vima-linux-x86_64 -o vima
chmod +x vima && sudo mv vima /usr/local/bin/

# Linux arm64
curl -fsSL https://github.com/pheuberger/vima/releases/latest/download/vima-linux-arm64 -o vima
chmod +x vima && sudo mv vima /usr/local/bin/

# macOS Apple Silicon
curl -fsSL https://github.com/pheuberger/vima/releases/latest/download/vima-darwin-arm64 -o vima
chmod +x vima && sudo mv vima /usr/local/bin/

# macOS Intel
curl -fsSL https://github.com/pheuberger/vima/releases/latest/download/vima-darwin-x86_64 -o vima
chmod +x vima && sudo mv vima /usr/local/bin/
```

### From source (global)

```sh
cargo install --git https://github.com/pheuberger/vima
```

## Where to initialize

Initialize vima **above** your project's source tree — not inside it. This keeps ticket data out of your source repo and works naturally with multiple git worktrees:

```
~/work/
  .vima/              # vima init here
  my-project/         # worktree 1 — vima finds .vima/ in parent
  my-project-wt/      # worktree 2 — same .vima/, shared tickets
```

vima walks up the directory tree from your cwd to find `.vima/`, so any subdirectory will find the store automatically. You can also set `VIMA_DIR` to point to an explicit location:

```sh
export VIMA_DIR=~/work/.vima
```

## Quick start

```sh
# Initialize a vima store (run from the parent of your project)
cd ~/work && vima init

# Create tickets
vima create "Set up CI pipeline" -t task -p 1
vima create "Login page returns 500" -t bug -p 0 --tags auth,urgent
vima create "Add dark mode" -t feature -p 3 --tags ui

# See what's ready to work on
vima ready

# Start working
vima start ID

# Close when done
vima close ID --reason "Deployed in v1.2"
```

## Commands

### Creating tickets

```sh
vima create "Title" [options]
```

| Flag | Description |
|------|-------------|
| `-t, --type TYPE` | `task`, `bug`, `feature`, `epic`, `chore` (default: `task`) |
| `-p, --priority N` | `0`=critical, `1`=high, `2`=medium (default), `3`=low, `4`=backlog |
| `-a, --assignee NAME` | Assignee |
| `-e, --estimate MINS` | Estimate in minutes |
| `--tags foo,bar` | Comma-separated tags |
| `--description TEXT` | Description |
| `--design TEXT` | Design notes |
| `--acceptance TEXT` | Acceptance criteria |
| `--dep ID` | Add dependency (repeatable) |
| `--blocks ID` | This ticket blocks ID (repeatable) |
| `--parent ID` | Parent ticket |
| `--id ID` | Explicit ID (otherwise auto-generated) |

### Listing tickets

```sh
vima list [options]       # Open tickets (default)
vima ready [options]      # Tickets with no open dependencies
vima blocked [options]    # Tickets with open dependencies
vima closed [options]     # Recently closed tickets (default limit: 20)
```

All list commands share these filters:

| Flag | Description |
|------|-------------|
| `-t, --type TYPE` | Filter by type |
| `-p, --priority RANGE` | Filter by priority (`2` or `0-2`) |
| `-T, --tag TAG` | Filter by tag (repeatable, OR semantics) |
| `-a, --assignee NAME` | Filter by assignee |
| `--status STATUS` | Filter by status (`open`, `in_progress`, `closed`) |
| `--limit N` | Limit results |
| `--pluck FIELD` | Extract field(s): `--pluck id` or `--pluck id,title` |
| `--count` | Print count only |

Results are sorted by priority (ascending), then by ID.

### Viewing a ticket

```sh
vima show ID
vima show ID --pluck title
```

### Updating tickets

```sh
vima update ID [options]
```

Accepts `--title`, `--description`, `--design`, `--acceptance`, `-p/--priority`, `--tags`, `-a/--assignee`, `-e/--estimate`, `--status`, `-t/--type`.

### Lifecycle

```sh
vima start ID                  # open -> in_progress
vima close ID [--reason "..."] # -> closed (adds note with reason)
vima close ID1 ID2 ID3         # close multiple at once
vima reopen ID                 # closed -> open
```

### Notes

```sh
vima add-note ID "Note text"
echo "Note from stdin" | vima add-note ID
```

### Dependencies

```sh
vima dep add ID DEP_ID           # ID depends on DEP_ID
vima dep add ID DEP_ID --blocks  # ID blocks DEP_ID (reverse)
vima undep ID DEP_ID             # Remove dependency
vima dep tree ID                 # Show dependency tree
vima dep tree ID --full          # Full transitive tree (allow dupes)
vima dep cycle                   # Detect cycles (exits 2 if found)
vima is-ready ID                 # Exits 0 if ready, non-zero if blocked
```

Cycle detection runs automatically when adding dependencies. A dependency that would create a cycle is rejected.

### Links

```sh
vima link ID_A ID_B              # Bidirectional link
vima unlink ID_A ID_B            # Remove link
```

## Output format

All commands emit **newline-delimited JSON** to stdout. Errors are JSON on stderr.

```sh
# Full ticket JSON
vima show my-abc1
# {"id":"my-abc1","title":"Fix login","status":"open","type":"bug",...}

# Extract a single field
vima list --pluck id
# "my-abc1"
# "my-xyz2"

# Extract multiple fields
vima list --pluck id,title
# {"id":"my-abc1","title":"Fix login"}

# Count
vima list --count
# 3

# Human-readable output
vima list --pretty
```

### Error output (stderr)

```json
{"error":"not_found","message":"ticket not found: xyz"}
{"error":"ambiguous_id","message":"ambiguous id 'ab': matches ab-1234, ab-5678","matches":["ab-1234","ab-5678"]}
{"error":"cycle","message":"dependency cycle detected: a -> b -> a","cycle":["a","b","a"]}
```

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | General error (invalid field, IO, parse, etc.) |
| `2` | Cycle detected (`dep cycle`) or ticket blocked (`is-ready`) |
| `3` | Not found or ambiguous ID |
| `4` | Conflict (ID already exists) |

## Batch create

Create multiple tickets from JSON on stdin, with back-references to link them:

```sh
vima create --batch <<'EOF'
[
  {"title": "Design API", "type": "task", "priority": 1, "id": "design"},
  {"title": "Implement API", "dep": ["$1"], "type": "task"},
  {"title": "Write tests", "dep": ["$2"], "type": "task"},
  {"title": "Deploy", "dep": ["$2", "$3"], "type": "task"}
]
EOF
```

`$1`, `$2`, etc. reference the IDs of previously created tickets (1-indexed). Available fields: `title`, `id`, `type`, `priority`, `assignee`, `estimate`, `tags`, `description`, `design`, `acceptance`, `dep`, `blocks`, `parent`.

## ID format

Ticket IDs follow the pattern `{prefix}-{4chars}`, e.g. `vi-2z5m`. The prefix is auto-derived from the project directory name or set in `.vima/config.yml`:

```yaml
prefix: vi
```

By default, vima uses **fuzzy ID matching** -- you can type any unique substring of an ID. Set `VIMA_EXACT=1` or pass `--exact` to require full ID matches.

## Storage

```
project/
  .vima/
    config.yml           # prefix: my
    tickets/
      my-a1b2.md
      my-c3d4.md
```

Each ticket is a Markdown file with YAML frontmatter:

```markdown
---
id: my-a1b2
title: Fix authentication timeout
status: open
type: bug
priority: 1
tags: [auth, backend]
assignee: alice
estimate: 120
deps: [my-c3d4]
links: []
parent: null
created: "2026-04-01T10:00:00Z"
notes:
  - timestamp: "2026-04-01T12:00:00Z"
    text: "Reproduced on staging"
---
Optional extended description in Markdown.
```

Tickets are plain files -- they diff, merge, and review in pull requests like any other code.

## Plugins

Extend vima by placing executables named `vima-{command}` on your `PATH`:

```sh
#!/bin/sh
# vima-plugin: Generate a weekly summary report
vima list --pluck id,title,status | jq -r '...'
```

The first 10 lines are scanned for `# vima-plugin: DESCRIPTION` to populate `vima help`.

Plugins receive these environment variables:

| Variable | Description |
|----------|-------------|
| `VIMA_DIR` | Path to `.vima/` directory |
| `VIMA_TICKETS_DIR` | Path to `.vima/tickets/` |
| `VIMA_BIN` | Path to the `vima` binary |

## Agent integration

vima is designed **agent-first**: AI agents (Claude Code, Cursor, Copilot, Devin, etc.) are the primary user. Every design decision optimizes for machine consumption — structured output, deterministic behavior, token efficiency, and zero interactivity.

### Discovery

Agents discover vima's full command schema at runtime:

```sh
vima help --json
```

This returns structured JSON with every command, flag, positional argument, subcommand, and description — everything an agent needs to construct valid invocations without guessing. The schema always reflects the installed version, so documentation never goes stale.

Add this to your project's `CLAUDE.md`, `AGENTS.md`, or equivalent:

```
This project uses `vima` for ticket tracking. Run `vima help --json` for the full command schema.
```

### Why agents prefer vima

| Property | How it helps agents |
|----------|-------------------|
| **JSON everywhere** | All output is structured NDJSON — parseable without regex, composable with `jq` |
| **`help --json`** | Runtime-discoverable schema — agents self-serve instead of reading docs |
| **Deterministic exit codes** | Branch on exit code without parsing output text |
| **`--exact` mode** | `VIMA_EXACT=1` prevents fuzzy matching surprises in automation |
| **`--pluck` and `--count`** | Extract exactly the data needed — minimizes token consumption |
| **`--full` opt-in** | Heavy fields (description, notes, body) excluded by default, saving tokens |
| **Batch create** | Build entire ticket graphs in one command with `$1`, `$2` back-references |
| **No interactive prompts** | Every operation is a single non-interactive command — no TTY required |
| **Structured errors** | JSON errors on stderr with `error` type, `message`, and context fields |
| **Atomic writes** | Crash-safe via temp-file-then-rename — safe for concurrent agent use |
| **File-based storage** | Tickets are plain Markdown — diff, merge, and review in PRs like code |

### Token-efficient patterns

Agents pay per token. Use these patterns to minimize context window consumption:

```sh
# Bad: dumps all fields for all tickets (hundreds of tokens per ticket)
vima list

# Good: only what you need (< 20 tokens per ticket)
vima list --pluck id,title,status

# Best: just check if there's work (1 token)
vima list --count

# Only load heavy fields when you actually need them
vima show ID                   # includes description, notes
vima list --pluck id           # lightweight scan
vima list --full               # opt-in to heavy fields in lists
```

### Agent workflow example

```sh
# Check what's ready to work on
NEXT=$(vima ready --pluck id --limit 1)

# Start it
vima start "$NEXT"

# ... do work ...

# Add a note about what was done
vima add-note "$NEXT" "Implemented in commit abc123"

# Close it
vima close "$NEXT" --reason "Shipped in PR #42"
```

### Distributed sync (`vima-sync` plugin)

Multiple agents and worktrees on the same machine already work — they share the same `.vima/` directory, with file locking and optimistic concurrency handling local races. The `vima-sync` plugin solves a different problem: **distributed collaboration where participants don't share a filesystem** — remote teammates, agents running in cloud sandboxes, CI systems, or developers on different machines.

It wraps any vima command with git pull/push, so participants get coordination without knowing about git:

```sh
# Instead of 5 chained git+vima commands, one command:
vima sync start vi-1234 --assignee agent-a

# Read-only commands pull fresh state automatically:
vima sync list --pluck id,title,status

# All mutation commands (create, update, start, close, reopen, delete, batch)
# are pulled, committed, and pushed atomically with retry logic.
```

If two participants race to claim the same ticket, one succeeds and the other gets exit code 6 (already claimed). Push failures trigger automatic retry with conflict detection (up to 3 attempts by default).

Configuration via environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `VIMA_SYNC_REMOTE` | `origin` | Git remote to sync with |
| `VIMA_SYNC_BRANCH` | current branch | Branch to push to |
| `VIMA_SYNC_RETRIES` | `3` | Max push retry attempts |

Install by placing `plugins/vima-sync` on your PATH:

```sh
cp plugins/vima-sync /usr/local/bin/
# or
export PATH="/path/to/vima-cli/plugins:$PATH"
```

### Multi-agent / automation setup

```sh
# Prevent fuzzy matching surprises in automation
export VIMA_EXACT=1

# Point to a shared store across worktrees
export VIMA_DIR=~/work/.vima

# Batch-create a dependency graph atomically
vima create --batch <<'EOF'
[
  {"title": "Design API", "type": "task", "priority": 1, "id": "design"},
  {"title": "Implement API", "dep": ["$1"]},
  {"title": "Write tests", "dep": ["$2"]},
  {"title": "Deploy", "dep": ["$2", "$3"]}
]
EOF
```

## Development

```sh
# Run tests
cargo test

# Format
cargo fmt

# Lint
cargo clippy -- -D warnings

# Build release binary (optimized, stripped)
cargo build --release
```

## License

See [LICENSE](LICENSE) for details.
