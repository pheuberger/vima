---
description: "Cut a new GitHub release for vima: bump Cargo.toml, build, tag, push, and run `gh release create` with notes generated from commits since the last tag. Use when user says 'create a release', 'cut a release', 'new gh release', or invokes /create-release."
---

# Create Release

Cut a new GitHub release for the vima Rust crate. Default mode is **fully automatic**: detect bump, build, commit, tag, push, publish — no questions unless something is ambiguous or risky.

## Input

Optional args:
- Explicit version: `0.3.0`, `v0.3.0`, or bump keyword `major|minor|patch`
- `--dry-run`: print plan, don't execute

If no args: auto-detect bump from commits since last tag (see step 3).

## Preconditions (fail fast)

Run in parallel; abort with clear error if any fail:

1. **On `main`**: `git rev-parse --abbrev-ref HEAD` must equal `main`. If not, stop and tell user.
2. **Clean working tree**: `git status --porcelain` must be empty. If dirty, stop — let user commit or stash.
3. **Up to date with remote**: `git fetch origin && git rev-list HEAD...origin/main --count` must be 0. If behind, stop.
4. **`gh` authenticated**: `gh auth status` must succeed.
5. **No existing tag for target version** (checked after version chosen).

## Workflow

### 1. Gather state

Run in parallel:
- `git tag --sort=-v:refname | head -5` — latest tags
- `git describe --tags --abbrev=0` — latest tag
- `git log $(git describe --tags --abbrev=0)..HEAD --oneline` — commits since
- `grep '^version' Cargo.toml | head -1` — current Cargo version
- `gh release list --limit 5` — existing releases

If no prior tag: this is the first release. Use Cargo.toml version as-is unless user overrides; tag `v<version>`.

### 2. Determine new version

If user passed an explicit version → use it.

If user passed `major|minor|patch` → bump latest tag accordingly.

Otherwise auto-detect from conventional-commit prefixes in commits since last tag:
- Any `feat!:`, `fix!:`, or `BREAKING CHANGE` in body → **major** (but if current major is `0`, bump **minor** instead — pre-1.0 convention)
- Any `feat:` → **minor**
- Only `fix:` / `chore:` / `docs:` / `refactor:` / `test:` / `style:` → **patch**
- Mixed but no `feat`/`!` → **patch**
- No commits since last tag → abort, nothing to release

Sanity check: new version must be greater than both latest tag and current Cargo.toml version. If not, abort.

### 3. Confirm (only if non-trivial)

Skip confirmation when:
- User passed explicit version or bump keyword
- Auto-bump is `patch`

Confirm when:
- Auto-bump is `minor` or `major` AND user gave no args. Show: latest tag, new version, one-line summary of commit categories. One yes/no question.

### 4. Bump Cargo.toml

Edit `Cargo.toml` `version = "X.Y.Z"` line. There is exactly one `[package]` version line — don't touch dependency versions.

### 5. Rebuild to refresh Cargo.lock

```bash
cargo build --release
```

If build fails: abort, surface the error, do not commit. Do **not** skip this — `Cargo.lock` must reflect the new version or CI/consumers see drift.

Tests are not run here (CI runs them on push). If user wants tests, they pass `--test` (not currently supported — just run `cargo test` manually first).

### 6. Commit, tag, push

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to <X.Y.Z>"
git tag v<X.Y.Z>
git push origin main
git push origin v<X.Y.Z>
```

If push to `main` is rejected (someone else pushed): abort with clear message. Do not force-push.

### 7. Generate release notes

Group commits since previous tag by conventional-commit prefix:

- `feat:` → **## Features**
- `fix:` → **## Fixes**
- `refactor:` → **## Refactors** (omit section if empty)
- `docs:` → **## Docs** (omit if empty)
- `perf:` → **## Performance** (omit if empty)
- `chore:` / `style:` / `test:` → omit entirely (noise)
- The `chore: bump version` commit itself → always omit

For each commit, strip the prefix and write one bullet. Rewrite terse messages into reviewer-friendly phrases when obvious; preserve user intent. Bold the headline noun where helpful (see prior `v0.2.0` notes for tone).

If a section has only one bullet, keep it — don't merge sections.

### 8. Publish

```bash
gh release create v<X.Y.Z> --title "v<X.Y.Z>" --notes "$(cat <<'EOF'
<notes>
EOF
)"
```

### 9. Report

One-line output:
```
Released v<X.Y.Z>: <release URL>
```

## Dry-run

If `--dry-run` passed: print the planned version, the diff for `Cargo.toml`, the commit/tag/push commands, and the rendered release notes. Make zero changes.

## Rules

- **Never release from a non-`main` branch**
- **Never release with a dirty tree** — abort, let user resolve
- **Never force-push** tags or `main`
- **Never skip the `cargo build --release`** — Cargo.lock must update
- **Never reuse an existing tag** — if `v<X.Y.Z>` exists, abort and ask user
- **Never invent commit categories** — if a commit has no conventional prefix, drop it into a generic **## Changes** section at the bottom
- **Pre-1.0 (`0.x.y`)**: breaking changes bump **minor**, not major. Only bump to `1.0.0` on explicit user request.
- **Cargo.toml is source of truth for version** — keep tag, Cargo.toml, and release in lockstep
