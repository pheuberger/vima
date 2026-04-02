# vima — ticket tracker

`vima` is this project's ticket tracker. Tickets live in `.vima/tickets/`.

Run `vima help --json` for the full command schema.

## Common commands

```
vima create "Title" [-t task|bug|feature] [-p 0-4] [--dep ID] [--tags foo,bar]
vima list [--tag foo] [--type bug] [--priority 0-2]
vima ready                    # tickets with no open deps
vima show ID
vima update ID --title "..." --description "..."
vima close ID [--reason "..."]
vima start ID                 # set status → in_progress
```

## Output format

All output is newline-delimited JSON (one object per line). Use `--pluck FIELD`
to extract a single field and `--count` to get a count.

```
vima list --pluck id          # print IDs only
vima list --count             # print number of open tickets
```

## Batch create with back-references

```
vima create --batch <<'EOF'
[
  {"title": "Task A", "id": "a"},
  {"title": "Task B", "dep": ["a"]}
]
EOF
```

## Dependencies

```
vima dep add ID DEP_ID        # ID depends on DEP_ID
vima dep add ID DEP_ID --blocks  # ID blocks DEP_ID
vima is-ready ID              # exits 0 if ready, 2 if blocked
```

## Automation tips

- Set `VIMA_EXACT=1` (or `--exact`) to disable partial ID matching.
- All commands exit 0 on success, non-zero on error.
- Errors are JSON on stderr with `error`, `message`, and context fields.
