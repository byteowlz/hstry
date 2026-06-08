#!/usr/bin/env bash
# Manual test: index hstry sessions into a mmry store (one entry per session).
#
# Env vars:
#   STORE      mmry store to write to     (default: hstry-sessions)
#   LIMIT      max sessions to ingest     (default: 20)
#   MODE       quick | full               (default: quick)
#                quick = title + first_user_message (no per-session show call)
#                full  = title + user/assistant text from hstry show, truncated
#   MSG_CHARS  per-message char cap (full mode only)  (default: 2000)
#   MAX_CHARS  total body char cap  (full mode only)  (default: 8000)
#   DRY_RUN    1 to print payloads only   (default: 0)
#
# Examples:
#   ./scripts/test-mmry-session-ingest.sh
#   LIMIT=50 MODE=full ./scripts/test-mmry-session-ingest.sh
#   DRY_RUN=1 LIMIT=3 ./scripts/test-mmry-session-ingest.sh
#
# After ingestion:
#   mmry --store hstry-sessions search "your topic" --mode hybrid

set -euo pipefail

STORE="${STORE:-hstry-sessions}"
LIMIT="${LIMIT:-20}"
MODE="${MODE:-quick}"
MSG_CHARS="${MSG_CHARS:-2000}"
MAX_CHARS="${MAX_CHARS:-8000}"
DRY_RUN="${DRY_RUN:-0}"

command -v jq >/dev/null || { echo "jq required" >&2; exit 1; }
command -v hstry >/dev/null || { echo "hstry required" >&2; exit 1; }
[[ "$DRY_RUN" == "1" ]] || command -v mmry >/dev/null || { echo "mmry required" >&2; exit 1; }

list=$(hstry --json list --limit "$LIMIT")
count=$(jq '.result | length' <<<"$list")
echo "Ingesting $count sessions into store '$STORE' (mode=$MODE, dry_run=$DRY_RUN)" >&2

jq -c '.result[]' <<<"$list" | while IFS= read -r s; do
  id=$(jq -r '.id' <<<"$s")
  title=$(jq -r '.title // "(untitled)"' <<<"$s")
  workspace=$(jq -r '.workspace // ""' <<<"$s")
  source_id=$(jq -r '.source_id // ""' <<<"$s")
  harness=$(jq -r '.harness // ""' <<<"$s")
  created=$(jq -r '.created_at // ""' <<<"$s")
  msg_count=$(jq -r '.message_count // 0' <<<"$s")

  if [[ "$MODE" == "full" ]]; then
    # Slice inside jq so nothing closes the pipe early (head -c would SIGPIPE
    # jq/hstry and trip pipefail). Tolerate per-session failures with || body=""
    # so one bad session doesn't kill the whole ingest.
    body=$(hstry --json show "$id" 2>/dev/null \
      | jq -r --argjson mc "$MSG_CHARS" --argjson mx "$MAX_CHARS" '
          [ .result.messages[]?
            | select(.role == "user" or .role == "assistant")
            | "[" + .role + "] " + ((.content // "")[0:$mc])
          ] | join("\n") | .[0:$mx]
        ' 2>/dev/null) || body=""
  else
    body=$(jq -r '.first_user_message // ""' <<<"$s")
  fi

  content=$(printf '%s\n\n%s' "$title" "$body")

  payload=$(jq -n \
    --arg content "$content" \
    --arg cat "$workspace" \
    --arg id "$id" \
    --arg src "$source_id" \
    --arg har "$harness" \
    --arg created "$created" \
    --arg title "$title" \
    --argjson mc "$msg_count" \
    '{
      content: $content,
      memory_type: "episodic",
      category: $cat,
      tags: [
        "hstry-session",
        ("source:" + $src),
        ("harness:" + $har),
        ("conv:" + $id)
      ],
      metadata: {
        hstry_session: {
          conversation_id: $id,
          title: $title,
          source_id: $src,
          harness: $har,
          workspace: $cat,
          created_at: $created,
          message_count: $mc
        }
      },
      importance: 5
    }')

  if [[ "$DRY_RUN" == "1" ]]; then
    echo "$payload"
  else
    echo "  -> $id  $title" >&2
    printf '%s' "$payload" | mmry add --store "$STORE" -
  fi
done

echo "Done." >&2
