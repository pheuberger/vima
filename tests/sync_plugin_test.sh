#!/usr/bin/env bash
# Integration tests for vima-sync plugin
# Requires: git, vima on PATH
#
# Creates temporary git repos (bare remote + two clones simulating two agents)
# and exercises the sync plugin's pull/push/conflict behavior.

set -euo pipefail

PASS=0
FAIL=0
TESTS_RUN=0

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PLUGIN_PATH="$(cd "$SCRIPT_DIR/../plugins" && pwd)/vima-sync"

# Colors for output (stderr only)
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

pass() {
    PASS=$((PASS + 1))
    TESTS_RUN=$((TESTS_RUN + 1))
    echo -e "${GREEN}PASS${NC}: $1" >&2
}

fail() {
    FAIL=$((FAIL + 1))
    TESTS_RUN=$((TESTS_RUN + 1))
    echo -e "${RED}FAIL${NC}: $1" >&2
    if [ -n "${2:-}" ]; then
        echo "  detail: $2" >&2
    fi
}

# --- setup ------------------------------------------------------------------

TMPDIR_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_ROOT"' EXIT

setup_repos() {
    local test_name="$1"
    local base="$TMPDIR_ROOT/$test_name"
    mkdir -p "$base"

    # Create bare remote
    git init --bare "$base/remote.git" --quiet

    # Clone for agent A
    git clone "$base/remote.git" "$base/agent-a" --quiet 2>/dev/null
    cd "$base/agent-a"
    git config user.email "test@test.com"
    git config user.name "Test"
    mkdir -p .vima/tickets
    echo "prefix: vi" > .vima/config.yml
    git add .vima/
    git commit -m "init vima" --quiet
    git push --quiet 2>/dev/null

    # Clone for agent B
    git clone "$base/remote.git" "$base/agent-b" --quiet 2>/dev/null
    cd "$base/agent-b"
    git config user.email "test@test.com"
    git config user.name "Test"

    echo "$base"
}

run_sync() {
    # Run the sync plugin with VIMA_DIR pointing to the right .vima/
    local agent_dir="$1"
    shift
    VIMA_DIR="$agent_dir/.vima" \
    VIMA_BIN="vima" \
    VIMA_SYNC_REMOTE="origin" \
    PATH="$(dirname "$PLUGIN_PATH"):$PATH" \
    bash "$PLUGIN_PATH" "$@"
}

# --- tests ------------------------------------------------------------------

test_basic_create_syncs() {
    local base
    base="$(setup_repos "basic_create")"

    # Agent A creates a ticket via sync
    cd "$base/agent-a"
    local output
    output=$(run_sync "$base/agent-a" create "Test ticket" -p 2 2>/dev/null) || {
        fail "basic create syncs" "sync create failed"
        return
    }

    # Verify the commit was pushed
    cd "$base/agent-b"
    git pull --quiet 2>/dev/null
    local ticket_count
    ticket_count=$(ls .vima/tickets/*.md 2>/dev/null | wc -l)
    if [ "$ticket_count" -eq 1 ]; then
        pass "basic create syncs to remote"
    else
        fail "basic create syncs to remote" "expected 1 ticket in agent-b, got $ticket_count"
    fi
}

test_read_only_pulls_without_push() {
    local base
    base="$(setup_repos "read_only")"

    # Agent A creates a ticket directly (no sync)
    cd "$base/agent-a"
    vima create "Direct ticket" -p 2 2>/dev/null
    git add .vima/ && git commit -m "add ticket" --quiet && git push --quiet 2>/dev/null

    # Agent B lists via sync — should see the ticket after pull
    cd "$base/agent-b"
    local output
    output=$(run_sync "$base/agent-b" list --pluck id 2>/dev/null) || {
        fail "read-only pulls without push" "sync list failed"
        return
    }

    if echo "$output" | grep -q 'vi-'; then
        pass "read-only list pulls fresh state"
    else
        fail "read-only list pulls fresh state" "output: $output"
    fi
}

test_start_claim_visible_to_other_agent() {
    local base
    base="$(setup_repos "claim_visible")"

    # Agent A creates a ticket
    cd "$base/agent-a"
    local create_out
    create_out=$(run_sync "$base/agent-a" create "Claimable ticket" -p 2 2>/dev/null)
    local ticket_id
    ticket_id=$(echo "$create_out" | grep -o '"id":"[^"]*"' | head -1 | cut -d'"' -f4)

    if [ -z "$ticket_id" ]; then
        fail "start claim visible" "could not extract ticket ID from create output"
        return
    fi

    # Agent A claims it
    run_sync "$base/agent-a" start "$ticket_id" --assignee agent-a 2>/dev/null || {
        fail "start claim visible" "sync start failed"
        return
    }

    # Agent B pulls and sees the claim
    cd "$base/agent-b"
    git pull --quiet 2>/dev/null
    local show_out
    show_out=$(vima show "$ticket_id" --pluck assignee 2>/dev/null)

    if echo "$show_out" | grep -q 'agent-a'; then
        pass "start claim visible to other agent"
    else
        fail "start claim visible to other agent" "expected assignee agent-a, got: $show_out"
    fi
}

test_preserves_exit_codes() {
    local base
    base="$(setup_repos "exit_codes")"
    cd "$base/agent-a"

    # Try to show a nonexistent ticket — should get exit 3
    local exit_code=0
    run_sync "$base/agent-a" show "vi-nonexistent" 2>/dev/null || exit_code=$?

    if [ "$exit_code" -eq 3 ]; then
        pass "preserves exit code 3 (not found)"
    else
        fail "preserves exit code 3 (not found)" "expected 3, got $exit_code"
    fi
}

test_no_command_shows_error() {
    local base
    base="$(setup_repos "no_cmd")"
    cd "$base/agent-a"

    local exit_code=0
    local stderr_out
    stderr_out=$(VIMA_DIR="$base/agent-a/.vima" VIMA_BIN="vima" bash "$PLUGIN_PATH" 2>&1) || exit_code=$?

    if [ "$exit_code" -ne 0 ] && echo "$stderr_out" | grep -q "no command"; then
        pass "no command shows error"
    else
        fail "no command shows error" "exit=$exit_code stderr=$stderr_out"
    fi
}

test_mutation_creates_commit() {
    local base
    base="$(setup_repos "commit_check")"
    cd "$base/agent-a"

    run_sync "$base/agent-a" create "Commit test" -p 3 2>/dev/null

    # Check that the latest commit message starts with "vima: create"
    local last_msg
    last_msg=$(git log -1 --format="%s")

    if echo "$last_msg" | grep -q "^vima: create"; then
        pass "mutation creates descriptive commit"
    else
        fail "mutation creates descriptive commit" "last commit: $last_msg"
    fi
}

test_dry_run_does_not_push() {
    local base
    base="$(setup_repos "dry_run")"
    cd "$base/agent-a"

    # Get commit count before
    local before
    before=$(git rev-list --count HEAD)

    run_sync "$base/agent-a" create "Dry run test" -p 3 --dry-run 2>/dev/null || true

    local after
    after=$(git rev-list --count HEAD)

    if [ "$before" -eq "$after" ]; then
        pass "dry-run does not create commit or push"
    else
        fail "dry-run does not create commit or push" "commits before=$before after=$after"
    fi
}

test_close_syncs() {
    local base
    base="$(setup_repos "close_sync")"
    cd "$base/agent-a"

    # Create and start a ticket
    local create_out
    create_out=$(run_sync "$base/agent-a" create "Closeable ticket" -p 2 2>/dev/null)
    local ticket_id
    ticket_id=$(echo "$create_out" | grep -o '"id":"[^"]*"' | head -1 | cut -d'"' -f4)

    run_sync "$base/agent-a" start "$ticket_id" 2>/dev/null
    run_sync "$base/agent-a" close "$ticket_id" 2>/dev/null

    # Agent B pulls and checks status
    cd "$base/agent-b"
    git pull --quiet 2>/dev/null
    local status
    status=$(vima show "$ticket_id" --pluck status 2>/dev/null)

    if echo "$status" | grep -q "closed"; then
        pass "close syncs to remote"
    else
        fail "close syncs to remote" "expected closed, got: $status"
    fi
}

test_env_var_defaults() {
    # Test that the plugin works without any VIMA_SYNC_* vars set
    local base
    base="$(setup_repos "env_defaults")"
    cd "$base/agent-a"

    # Unset all VIMA_SYNC vars and run
    local output
    output=$(VIMA_DIR="$base/agent-a/.vima" \
             VIMA_BIN="vima" \
             bash -c 'unset VIMA_SYNC_REMOTE VIMA_SYNC_BRANCH VIMA_SYNC_RETRIES; bash '"$PLUGIN_PATH"' create "Env test" -p 3' 2>/dev/null) || {
        fail "env var defaults" "failed without VIMA_SYNC_* vars"
        return
    }

    if echo "$output" | grep -q '"id"'; then
        pass "works with default env vars"
    else
        fail "works with default env vars" "output: $output"
    fi
}

# --- run all tests ----------------------------------------------------------

echo "Running vima-sync plugin tests..." >&2
echo "---" >&2

test_basic_create_syncs
test_read_only_pulls_without_push
test_start_claim_visible_to_other_agent
test_preserves_exit_codes
test_no_command_shows_error
test_mutation_creates_commit
test_dry_run_does_not_push
test_close_syncs
test_env_var_defaults

echo "---" >&2
echo "Results: $PASS passed, $FAIL failed, $TESTS_RUN total" >&2

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
