#!/usr/bin/env bash
# End-to-end test for all hstry adapters
# Tests parsing fixtures and cross-format conversion

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
HSTRY="$PROJECT_ROOT/target/debug/hstry"
TEST_DB="$SCRIPT_DIR/test.db"
TEST_CONFIG="$SCRIPT_DIR/test-config.toml"
EXPORT_DIR="$SCRIPT_DIR/export-output"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

passed=0
failed=0

# Cleanup function
cleanup() {
    rm -f "$TEST_DB" "$TEST_DB-shm" "$TEST_DB-wal"
    rm -rf "$EXPORT_DIR"
}

# Run on exit
trap cleanup EXIT

log_pass() {
    echo -e "${GREEN}PASS${NC}: $1"
    passed=$((passed + 1))
}

log_fail() {
    echo -e "${RED}FAIL${NC}: $1"
    failed=$((failed + 1))
}

log_info() {
    echo -e "${YELLOW}INFO${NC}: $1"
}

# Build the project first
log_info "Building hstry..."
if ! (cd "$PROJECT_ROOT" && cargo build --bin hstry 2>/dev/null); then
    log_fail "Failed to build hstry"
    exit 1
fi

has_sqlite_support=false
if node -e "require('better-sqlite3')" >/dev/null 2>&1; then
    has_sqlite_support=true
else
    log_info "better-sqlite3 not available; skipping sqlite-based adapter assertions"
fi

# Ensure test config exists with correct paths
cat > "$TEST_CONFIG" << EOF
database = "$TEST_DB"

[[sources]]
id = "test-chatgpt"
adapter = "chatgpt"
path = "$SCRIPT_DIR/chatgpt/conversations.json"

[[sources]]
id = "test-claude-web"
adapter = "claude-web"
path = "$SCRIPT_DIR/claude-web/claude-conversations.json"

[[sources]]
id = "test-gemini"
adapter = "gemini"
path = "$SCRIPT_DIR/gemini/gemini-conversations.json"

[[sources]]
id = "test-opencode"
adapter = "opencode"
path = "$SCRIPT_DIR/opencode"

[[sources]]
id = "test-codex"
adapter = "codex"
path = "$SCRIPT_DIR/codex"

[[sources]]
id = "test-claude-code"
adapter = "claude-code"
path = "$SCRIPT_DIR/claude-code"

[[sources]]
id = "test-pi"
adapter = "pi"
path = "$SCRIPT_DIR/pi"

[[sources]]
id = "test-aider"
adapter = "aider"
path = "$SCRIPT_DIR/aider"

[[sources]]
id = "test-chatgpt-teams"
adapter = "chatgpt-teams"
path = "$SCRIPT_DIR/chatgpt-teams"

[[sources]]
id = "test-goose"
adapter = "goose"
path = "$SCRIPT_DIR/goose"

[[sources]]
id = "test-cursor"
adapter = "cursor"
path = "$SCRIPT_DIR/cursor"

[[sources]]
id = "test-jan"
adapter = "jan"
path = "$SCRIPT_DIR/jan/threads"

[[sources]]
id = "test-lmstudio"
adapter = "lmstudio"
path = "$SCRIPT_DIR/lmstudio"

[[sources]]
id = "test-openwebui"
adapter = "openwebui"
path = "$SCRIPT_DIR/openwebui"

[service]
enabled = false
poll_interval_secs = 30
EOF

log_info "Test config created at $TEST_CONFIG"

# Remove old test db if it exists
rm -f "$TEST_DB" "$TEST_DB-shm" "$TEST_DB-wal"

# Test 1: Sync all sources
log_info "Testing sync of all adapter fixtures..."
if "$HSTRY" --config "$TEST_CONFIG" sync 2>/dev/null; then
    log_pass "Sync completed successfully"
else
    log_fail "Sync failed"
fi

# Test 2: List conversations and verify count
log_info "Verifying imported conversations..."
conv_count=$("$HSTRY" --config "$TEST_CONFIG" list --json 2>/dev/null | jq '.result | length')
if [ "$conv_count" -ge 8 ]; then
    log_pass "Found $conv_count conversations (expected at least 8)"
else
    log_fail "Only found $conv_count conversations (expected at least 8)"
fi

# Test 3: Search for known content
log_info "Testing search across all formats..."
search_results=$("$HSTRY" --config "$TEST_CONFIG" search "Hello" --json 2>/dev/null | jq '.result | length')
if [ "$search_results" -ge 1 ]; then
    log_pass "Search found $search_results results for 'Hello'"
else
    log_fail "Search found no results for 'Hello'"
fi

# Test 4: Export to each format
log_info "Testing export to all formats..."
mkdir -p "$EXPORT_DIR"

export_formats=("json" "markdown" "pi" "opencode" "codex" "claude-code")
for fmt in "${export_formats[@]}"; do
    log_info "  Exporting to $fmt..."
    if "$HSTRY" --config "$TEST_CONFIG" export --format "$fmt" --output "$EXPORT_DIR/$fmt" --pretty 2>/dev/null; then
        log_pass "Export to $fmt succeeded"
    else
        log_fail "Export to $fmt failed"
    fi
done

# Test 5: Verify specific adapter parsing
log_info "Verifying specific adapter parsing..."

# ChatGPT
chatgpt_conv=$("$HSTRY" --config "$TEST_CONFIG" list --json --source test-chatgpt 2>/dev/null | jq '.result[0]')
if echo "$chatgpt_conv" | jq -e '.title == "Test ChatGPT Conversation"' >/dev/null 2>&1; then
    log_pass "ChatGPT adapter parsed title correctly"
else
    log_fail "ChatGPT adapter failed to parse title"
fi

# ChatGPT Teams
teams_conv=$("$HSTRY" --config "$TEST_CONFIG" list --json --source test-chatgpt-teams 2>/dev/null | jq '.result[0]')
if echo "$teams_conv" | jq -e '.title == "Test ChatGPT Teams Conversation"' >/dev/null 2>&1; then
    log_pass "ChatGPT Teams adapter parsed title correctly"
else
    log_fail "ChatGPT Teams adapter failed to parse title"
fi

# Claude Web
claude_conv=$("$HSTRY" --config "$TEST_CONFIG" list --json --source test-claude-web 2>/dev/null | jq '.result[0]')
if echo "$claude_conv" | jq -e '.title == "Test Claude Web Conversation"' >/dev/null 2>&1; then
    log_pass "Claude Web adapter parsed title correctly"
else
    log_fail "Claude Web adapter failed to parse title"
fi

# Gemini
gemini_conv=$("$HSTRY" --config "$TEST_CONFIG" list --json --source test-gemini 2>/dev/null | jq '.result[0]')
if echo "$gemini_conv" | jq -e '.title == "Test Gemini Conversation"' >/dev/null 2>&1; then
    log_pass "Gemini adapter parsed title correctly"
else
    log_fail "Gemini adapter failed to parse title"
fi

# OpenCode
opencode_conv=$("$HSTRY" --config "$TEST_CONFIG" list --json --source test-opencode 2>/dev/null | jq '.result[0]')
if echo "$opencode_conv" | jq -e '.title == "Test OpenCode Session"' >/dev/null 2>&1; then
    log_pass "OpenCode adapter parsed title correctly"
else
    log_fail "OpenCode adapter failed to parse title"
fi

# Goose
goose_conv=$("$HSTRY" --config "$TEST_CONFIG" list --json --source test-goose 2>/dev/null | jq '.result[0]')
if echo "$goose_conv" | jq -e '.title == "Hello Goose!"' >/dev/null 2>&1; then
    log_pass "Goose adapter parsed title correctly"
else
    log_fail "Goose adapter failed to parse title"
fi

# Pi
pi_conv=$("$HSTRY" --config "$TEST_CONFIG" list --json --source test-pi 2>/dev/null | jq '.result[0]')
if echo "$pi_conv" | jq -e '.title == "Test Pi Session"' >/dev/null 2>&1; then
    log_pass "Pi adapter parsed title correctly"
else
    log_fail "Pi adapter failed to parse title"
fi

# Jan
jan_conv=$("$HSTRY" --config "$TEST_CONFIG" list --json --source test-jan 2>/dev/null | jq '.result[0]')
if echo "$jan_conv" | jq -e '.title == "Test Jan Thread"' >/dev/null 2>&1; then
    log_pass "Jan adapter parsed title correctly"
else
    log_fail "Jan adapter failed to parse title"
fi

# LM Studio
lmstudio_conv=$("$HSTRY" --config "$TEST_CONFIG" list --json --source test-lmstudio 2>/dev/null | jq '.result[0]')
if echo "$lmstudio_conv" | jq -e '.title == "Test LM Studio Conversation"' >/dev/null 2>&1; then
    log_pass "LM Studio adapter parsed title correctly"
else
    log_fail "LM Studio adapter failed to parse title"
fi

# Aider
aider_conv=$("$HSTRY" --config "$TEST_CONFIG" list --json --source test-aider 2>/dev/null | jq '.result[0]')
if echo "$aider_conv" | jq -e '.title == "Test Aider Session"' >/dev/null 2>&1; then
    log_pass "Aider adapter parsed title correctly"
else
    log_fail "Aider adapter failed to parse title"
fi

# Codex
codex_conv=$("$HSTRY" --config "$TEST_CONFIG" list --json --source test-codex 2>/dev/null | jq '.result[0]')
if echo "$codex_conv" | jq -e '.external_id == "test-codex-session-1"' >/dev/null 2>&1; then
    log_pass "Codex adapter parsed session correctly"
else
    log_fail "Codex adapter failed to parse session"
fi

# Claude Code
claudecode_conv=$("$HSTRY" --config "$TEST_CONFIG" list --json --source test-claude-code 2>/dev/null | jq '.result[0]')
if echo "$claudecode_conv" | jq -e '.title == "Test Claude Code Session"' >/dev/null 2>&1; then
    log_pass "Claude Code adapter parsed title correctly"
else
    log_fail "Claude Code adapter failed to parse title"
fi

# Cursor
if [ "$has_sqlite_support" = "true" ]; then
    cursor_conv=$("$HSTRY" --config "$TEST_CONFIG" list --json --source test-cursor 2>/dev/null | jq '.result[0]')
    if echo "$cursor_conv" | jq -e '.title == "Test Cursor Chat"' >/dev/null 2>&1; then
        log_pass "Cursor adapter parsed title correctly"
    else
        log_fail "Cursor adapter failed to parse title"
    fi
else
    log_info "Skipping Cursor adapter checks"
fi

# Open WebUI
if [ "$has_sqlite_support" = "true" ]; then
    openwebui_conv=$("$HSTRY" --config "$TEST_CONFIG" list --json --source test-openwebui 2>/dev/null | jq '.result[0]')
    if echo "$openwebui_conv" | jq -e '.title == "Test Open WebUI Conversation"' >/dev/null 2>&1; then
        log_pass "Open WebUI adapter parsed title correctly"
    else
        log_fail "Open WebUI adapter failed to parse title"
    fi
else
    log_info "Skipping Open WebUI adapter checks"
fi

# Summary
echo ""
echo "=========================================="
echo -e "Test Results: ${GREEN}$passed passed${NC}, ${RED}$failed failed${NC}"
echo "=========================================="

if [ "$failed" -gt 0 ]; then
    exit 1
fi

exit 0
