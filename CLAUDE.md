# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project overview

vima is a file-based, **agent-first** ticket tracker. Tickets are Markdown files with YAML frontmatter stored in `.vima/tickets/`. All output is newline-delimited JSON. Written in Rust.

This project uses vima itself as its **only** ticket tracker — all work is tracked exclusively via the `vima` CLI (available on PATH). Do not use any other issue-tracking system. Always use `vima` commands to create, list, update, and close tickets. **Never read or write `.vima/tickets/` files directly** — use the CLI for everything.

## Agent-first design philosophy

vima is built for AI agents as the primary user. Every design decision optimizes for machine consumption first, human convenience second. When adding features or modifying behavior, apply these principles:

### Core principles

1. **Structured output always** — JSON to stdout, errors to stderr. Never mix human prose into stdout. Agents parse stdout; humans read stderr and `--pretty`.
2. **Token efficiency** — Every token an agent reads costs money and context window space. Use `--pluck` to return only needed fields. Default list output excludes heavy fields (description, notes, body); use `--full` to include them. Prefer concise field names.
3. **Deterministic behavior** — Same input must produce same output. No interactive prompts, no pagers, no color codes in stdout. Exit codes encode status for branching without parsing.
4. **Progressive disclosure** — `help --json` gives full schema; per-command help gives focused detail. Don't dump everything upfront. Agents should load only what they need.
5. **Idempotency where possible** — Agents retry. Design commands so repeated calls are safe or produce clear conflict signals (distinct exit codes).
6. **Actionable errors** — Error messages should include recovery suggestions. An error that says "not found" is less useful than one that says "not found — run `vima list --pluck id` to see available tickets".
7. **Batch-native** — Agents generate structured data naturally. Accept JSON on stdin for bulk operations. Support back-references for building graphs atomically.

### Anti-patterns to avoid

- Never add interactive prompts or TTY-dependent behavior to commands
- Never output unstructured prose to stdout (use stderr for human messages)
- Never add features that require parsing regex patterns from output
- Never break JSON output format for existing commands
- Never add flags that only make sense for human interactive use without a machine equivalent
- Never require multiple round-trips when a single command could suffice

### When designing new commands

- Default output should be JSON, parseable by `jq`
- Add `--pluck` and `--count` support to any list-like command
- Use semantic exit codes: 0=success, 1=error, 2=cycle/blocked
- Include the command in `help --json` schema automatically
- Write error types in `error.rs` with structured fields, not just messages
- Consider: "Can an agent use this without reading documentation first?"

## Commit conventions

Use **semantic commit messages**: `type: short description`. Common prefixes: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`.

## Testing policy

**Every code change MUST include tests.** No exceptions. New features, bug fixes, refactors — all require corresponding test coverage. Do not submit code without tests. Run `cargo test` to verify all tests pass before considering work complete.

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

**Context-efficient output** (minimize tokens consumed by agent):
```bash
vima list --pluck id          # IDs only — use this to check what exists
vima list --pluck id,title    # minimal summary
vima list --count             # just the count, no records
vima show ID --pluck status   # single field from single ticket
vima list --full              # include heavy fields only when needed
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
- Use `--pluck` aggressively to minimize output tokens
- Pipe `--pluck id` output to subsequent commands for workflows
- Use `--count` instead of piping to `wc -l`
