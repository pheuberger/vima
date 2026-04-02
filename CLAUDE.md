# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project overview

vima is a file-based, agent-first ticket tracker. Tickets are Markdown files with YAML frontmatter stored in `.vima/tickets/`. All output is newline-delimited JSON. Written in Rust.

This project uses vima itself for tracking its own work — check `.vima/tickets/` for open issues.

## Build & development commands

```bash
cargo build                          # debug build
cargo build --release                # release build (stripped, LTO)
cargo test                           # run all tests (~223 tests)
cargo test <test_name>               # run a single test by name
cargo test --lib store               # run tests in a specific module
cargo fmt                            # format code
cargo clippy -- -D warnings          # lint (CI enforces zero warnings)
```

CI runs: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.

## Architecture

**Entry point**: `src/main.rs` — CLI dispatcher that routes commands to handler functions. All command logic lives here (~3100 lines).

**Core modules**:
- `cli.rs` — Clap derive-based argument parsing for all commands
- `ticket.rs` — Data model: `Ticket`, `Note`, `Status`, `TicketType`, priority levels
- `store.rs` — Persistence: reads/writes YAML+MD files, temp-file-then-rename for crash safety
- `id.rs` — ID generation (`{prefix}-{4char}`), fuzzy resolution, validation
- `deps.rs` — Dependency graph: 3-color DFS cycle detection, tree building (dedup/full modes)
- `filter.rs` — Filtering (tags OR, priority range, status, type, assignee) and sorting
- `batch.rs` — Batch create from JSON with back-references (`$1`, `$2`, etc.)
- `output.rs` — JSON formatting, `--pluck` field extraction, `--count`, `--pretty` colored output
- `error.rs` — Structured error types with JSON serialization and exit codes (0=ok, 1=error, 2=cycle/blocked)
- `plugin.rs` — Discovers `vima-{name}` executables on PATH, passes context via env vars

**Key patterns**:
- Computed fields (`blocks`, `children`) are derived at read time from reverse lookups, not stored
- Fuzzy ID matching by default; `VIMA_EXACT=1` or `--exact` for automation
- Plugins get `VIMA_DIR`, `VIMA_TICKETS_DIR`, `VIMA_BIN` env vars and run via `exec()`

## Storage format

```
.vima/
  config.yml              # prefix: vi (2-4 char ID prefix)
  tickets/
    vi-xxxx.md            # one file per ticket (YAML frontmatter + optional MD body)
```

## Using vima (this project's tracker)

Run `vima help --json` for the full command schema.

```bash
vima create "Title" [-t task|bug|feature] [-p 0-4] [--dep ID] [--tags foo,bar]
vima list [--tag foo] [--type bug] [--priority 0-2]
vima ready                    # tickets with no open deps
vima show ID
vima update ID --title "..." --description "..."
vima close ID [--reason "..."]
vima start ID                 # set status → in_progress
```

**Output manipulation**:
```bash
vima list --pluck id          # print IDs only
vima list --count             # count of open tickets
```

**Dependencies**:
```bash
vima dep add ID DEP_ID        # ID depends on DEP_ID
vima dep add ID DEP_ID --blocks  # ID blocks DEP_ID
vima is-ready ID              # exits 0 if ready, 2 if blocked
```

**Batch create with back-references**:
```bash
vima create --batch <<'EOF'
[
  {"title": "Task A", "id": "a"},
  {"title": "Task B", "dep": ["a"]}
]
EOF
```

**Automation tips**:
- Set `VIMA_EXACT=1` (or `--exact`) to disable partial ID matching
- All commands exit 0 on success, non-zero on error
- Errors are JSON on stderr with `error`, `message`, and context fields
