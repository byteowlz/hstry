#!/usr/bin/env bash
# Eval recall quality across search backends for the "find the session I did X in" task.
#
# Reads queries with ground-truth conversation IDs from a JSON file, runs each
# query through a set of engines, computes per-query rank/RR/latency and per-engine
# MRR / Hit@K / mean latency. Prints comparison tables.
#
# Engines:
#   mmry-hybrid           hybrid mode with rerank
#   mmry-hybrid-norerank  hybrid mode, no rerank
#   mmry-semantic         dense embeddings only
#   mmry-bm25             BM25 only
#   mmry-sparse           SPLADE++ only
#   mmry-keyword          exact match
#   mmry-fuzzy            typo-tolerant
#   hstry-search          baseline: hstry's own message-level FTS
#
# Usage:
#   ./scripts/eval-mmry-recall.sh eval/queries.example.json
#   ./scripts/eval-mmry-recall.sh queries.json json > results.json
#   ENGINES=mmry-hybrid,mmry-semantic,hstry-search ./scripts/eval-mmry-recall.sh queries.json
#   STORE=hstry-sessions LIMIT=20 K_VALUES=1,3,10 ./scripts/eval-mmry-recall.sh queries.json

set -euo pipefail

queries_file="${1:-}"
output_mode="${2:-table}"   # table | json | csv

[[ -z "$queries_file" ]] && { echo "usage: $0 <queries.json> [table|json|csv]" >&2; exit 2; }
[[ ! -f "$queries_file" ]] && { echo "no such file: $queries_file" >&2; exit 2; }

STORE="${STORE:-hstry-sessions}"
LIMIT="${LIMIT:-20}"
K_VALUES="${K_VALUES:-1,3,5,10}"
ENGINES="${ENGINES:-mmry-hybrid,mmry-hybrid-norerank,mmry-semantic,mmry-bm25,mmry-sparse,mmry-keyword,mmry-fuzzy,hstry-search}"

IFS=',' read -ra ENGINE_LIST <<< "$ENGINES"
IFS=',' read -ra K_LIST <<< "$K_VALUES"

command -v jq >/dev/null || { echo "jq required" >&2; exit 1; }
command -v mmry >/dev/null || { echo "mmry required" >&2; exit 1; }
command -v hstry >/dev/null || { echo "hstry required" >&2; exit 1; }

# -------- engines --------

run_engine() {
  local engine="$1" query="$2"
  case "$engine" in
    mmry-hybrid)
      mmry --store "$STORE" search "$query" --mode hybrid --rerank --limit "$LIMIT" --json ;;
    mmry-hybrid-norerank)
      mmry --store "$STORE" search "$query" --mode hybrid --no-rerank --limit "$LIMIT" --json ;;
    mmry-semantic)
      mmry --store "$STORE" search "$query" --mode semantic --limit "$LIMIT" --json ;;
    mmry-bm25)
      mmry --store "$STORE" search "$query" --mode bm25 --limit "$LIMIT" --json ;;
    mmry-sparse)
      mmry --store "$STORE" search "$query" --mode sparse --limit "$LIMIT" --json ;;
    mmry-keyword)
      mmry --store "$STORE" search "$query" --mode keyword --limit "$LIMIT" --json ;;
    mmry-fuzzy)
      mmry --store "$STORE" search "$query" --mode fuzzy --limit "$LIMIT" --json ;;
    hstry-search)
      hstry --json search "$query" --limit "$LIMIT" ;;
    *)
      echo "unknown engine: $engine" >&2; return 1 ;;
  esac
}

# Extract ordered, deduped conversation IDs (one per line) from an engine's JSON.
extract_conv_ids() {
  local engine="$1"
  if [[ "$engine" == hstry-* ]]; then
    jq -r '.result[]?.conversation_id // empty' | awk 'NF && !seen[$0]++'
  else
    # mmry CLI may return an array directly, {memories:[...]}, or {results:[...]}.
    # mmry currently drops nested `metadata` on ingest, so we also fall back to
    # a `conv:<id>` tag (which the ingest script attaches and mmry preserves).
    jq -r '
      (if type=="array" then .
       elif has("memories") then .memories
       elif has("results") then .results
       else [] end)
      | .[]?
      | ( .metadata.hstry_session.conversation_id
          // ((.metadata_json // "{}") | (try fromjson catch null) | .hstry_session.conversation_id?)
          // ([.tags[]? | select(type == "string" and startswith("conv:")) | sub("^conv:"; "")][0])
          // empty )
    ' | awk 'NF && !seen[$0]++'
  fi
}

# Lowest 1-based rank at which any expected ID appears; 0 if none.
first_hit_rank() {
  local expected_json="$1"
  local hits_str="$2"
  jq -nr --argjson exp "$expected_json" --arg hits "$hits_str" '
    ($hits | split("\n") | map(select(length > 0))) as $h
    | ($exp | map(. // "")) as $e
    | [ $h | to_entries[] | select(.value | IN($e[])) | .key + 1 ] | (min // 0)
  '
}

now_ms() {
  if [[ -n "${EPOCHREALTIME:-}" ]]; then
    local s=${EPOCHREALTIME%.*}
    local us=${EPOCHREALTIME#*.}
    printf '%s' "$(( s * 1000 + 10#${us:0:3} ))"
  else
    date +%s%3N
  fi
}

# -------- run --------

declare -A E_RANKS E_LATS E_COUNT
total_queries=$(jq '.queries | length' "$queries_file")
results_json='{"meta":{},"queries":[]}'

echo "Eval: $total_queries queries × ${#ENGINE_LIST[@]} engines (store=$STORE, limit=$LIMIT)" >&2

q_idx=0
while read -r query_obj; do
  q_idx=$((q_idx + 1))
  name=$(jq -r '.name // (.query | .[0:40])' <<<"$query_obj")
  query=$(jq -r '.query' <<<"$query_obj")
  expected=$(jq -c '.expected // []' <<<"$query_obj")
  expected_count=$(jq 'length' <<<"$expected")

  if [[ "$expected_count" == "0" ]]; then
    echo "  skip: '$name' has no expected IDs" >&2
    continue
  fi

  if [[ "$output_mode" == "table" ]]; then
    echo
    echo "── [$q_idx/$total_queries] $name"
    echo "   query:    $query"
    echo "   expected: $(jq -r 'join(", ")' <<<"$expected")"
    printf '   %-22s %-6s %-6s %-9s %s\n' "engine" "rank" "RR" "latency" "top-3"
  fi

  per_engine='[]'
  for engine in "${ENGINE_LIST[@]}"; do
    t0=$(now_ms)
    raw=$(run_engine "$engine" "$query" 2>/dev/null || echo '{}')
    t1=$(now_ms)
    lat_ms=$((t1 - t0))

    hits=$(printf '%s' "$raw" | extract_conv_ids "$engine" || true)
    rank=$(first_hit_rank "$expected" "$hits")
    [[ -z "$rank" ]] && rank=0
    rr=$(awk -v r="$rank" 'BEGIN{ printf "%.3f", (r==0?0:1.0/r) }')
    top3=$(printf '%s' "$hits" | head -3 | awk 'BEGIN{ORS=","} {print substr($0,1,8)} END{print ""}' | sed 's/,$//; s/,$//')

    E_RANKS[$engine]+="$rank "
    E_LATS[$engine]+="$lat_ms "
    E_COUNT[$engine]=$((${E_COUNT[$engine]:-0} + 1))

    if [[ "$output_mode" == "table" ]]; then
      printf '   %-22s %-6s %-6s %-9s %s\n' "$engine" "$rank" "$rr" "${lat_ms}ms" "$top3"
    fi

    per_engine=$(jq -c \
      --arg eng "$engine" --argjson rank "$rank" --argjson rr "$rr" \
      --argjson lat "$lat_ms" --arg top3 "$top3" \
      '. + [{engine:$eng, rank:$rank, rr:$rr, latency_ms:$lat, top3:$top3}]' \
      <<<"$per_engine")
  done

  results_json=$(jq -c \
    --arg name "$name" --arg query "$query" \
    --argjson expected "$expected" --argjson results "$per_engine" \
    '.queries += [{name:$name, query:$query, expected:$expected, results:$results}]' \
    <<<"$results_json")
done < <(jq -c '.queries[]' "$queries_file")

# -------- aggregate + emit --------

print_summary_table() {
  echo
  echo "═══ SUMMARY (store=$STORE, limit=$LIMIT, N=${E_COUNT[${ENGINE_LIST[0]}]:-0} queries)"
  printf '%-22s %-5s %-6s' "engine" "N" "MRR"
  for k in "${K_LIST[@]}"; do printf ' %-8s' "hit@$k"; done
  printf ' %s\n' "mean_lat"
  echo "$(printf '%.0s─' {1..80})"

  # Compute MRR per engine for sorting
  local sortable=""
  for engine in "${ENGINE_LIST[@]}"; do
    local ranks="${E_RANKS[$engine]:-}"
    [[ -z "$ranks" ]] && continue
    local mrr
    mrr=$(awk -v r="$ranks" 'BEGIN{
      n=split(r,a," "); s=0; c=0;
      for(i=1;i<=n;i++) if(a[i]!=""){ c++; s += (a[i]+0>0 ? 1.0/(a[i]+0) : 0) }
      printf "%.4f", (c>0?s/c:0)
    }')
    sortable+="$mrr $engine"$'\n'
  done

  while IFS=' ' read -r mrr engine; do
    [[ -z "$engine" ]] && continue
    local ranks="${E_RANKS[$engine]}"
    local lats="${E_LATS[$engine]}"
    local n="${E_COUNT[$engine]}"
    local mean_lat
    mean_lat=$(awk -v l="$lats" 'BEGIN{
      n=split(l,a," "); s=0; c=0;
      for(i=1;i<=n;i++) if(a[i]!=""){ c++; s += a[i]+0 }
      printf "%d", (c>0?s/c:0)
    }')
    printf '%-22s %-5s %-6.3f' "$engine" "$n" "$mrr"
    for k in "${K_LIST[@]}"; do
      local hit
      hit=$(awk -v r="$ranks" -v k="$k" 'BEGIN{
        n=split(r,a," "); h=0; c=0;
        for(i=1;i<=n;i++) if(a[i]!=""){ c++; if(a[i]+0>0 && a[i]+0<=k+0) h++ }
        printf "%.2f", (c>0?h/c:0)
      }')
      printf ' %-8s' "$hit"
    done
    printf ' %sms\n' "$mean_lat"
  done < <(printf '%s' "$sortable" | sort -rn)
}

build_summary_json() {
  local summary='[]'
  for engine in "${ENGINE_LIST[@]}"; do
    local ranks="${E_RANKS[$engine]:-}"
    [[ -z "$ranks" ]] && continue
    local lats="${E_LATS[$engine]}"
    local n="${E_COUNT[$engine]}"
    local mrr
    mrr=$(awk -v r="$ranks" 'BEGIN{
      n=split(r,a," "); s=0; c=0;
      for(i=1;i<=n;i++) if(a[i]!=""){ c++; s += (a[i]+0>0 ? 1.0/(a[i]+0) : 0) }
      printf "%.4f", (c>0?s/c:0)
    }')
    local mean_lat
    mean_lat=$(awk -v l="$lats" 'BEGIN{
      n=split(l,a," "); s=0; c=0;
      for(i=1;i<=n;i++) if(a[i]!=""){ c++; s += a[i]+0 }
      printf "%d", (c>0?s/c:0)
    }')
    local hits_json='{}'
    for k in "${K_LIST[@]}"; do
      local hit
      hit=$(awk -v r="$ranks" -v k="$k" 'BEGIN{
        n=split(r,a," "); h=0; c=0;
        for(i=1;i<=n;i++) if(a[i]!=""){ c++; if(a[i]+0>0 && a[i]+0<=k+0) h++ }
        printf "%.4f", (c>0?h/c:0)
      }')
      hits_json=$(jq -c --arg k "hit@$k" --argjson v "$hit" '. + {($k): $v}' <<<"$hits_json")
    done
    summary=$(jq -c --arg eng "$engine" --argjson n "$n" --argjson mrr "$mrr" \
      --argjson mean_lat "$mean_lat" --argjson hits "$hits_json" \
      '. + [{engine:$eng, n:$n, mrr:$mrr, mean_lat_ms:$mean_lat, hits:$hits}]' \
      <<<"$summary")
  done
  printf '%s' "$summary"
}

case "$output_mode" in
  table)
    print_summary_table
    ;;
  json)
    summary=$(build_summary_json)
    jq -n \
      --arg store "$STORE" --argjson limit "$LIMIT" \
      --argjson queries "$(jq '.queries' <<<"$results_json")" \
      --argjson summary "$summary" \
      '{meta:{store:$store, limit:$limit}, queries:$queries, summary:$summary}'
    ;;
  csv)
    echo "query_name,query,engine,rank,rr,latency_ms"
    jq -r '.queries[] | . as $q | $q.results[] | [$q.name, $q.query, .engine, .rank, .rr, .latency_ms] | @csv' <<<"$results_json"
    ;;
  *)
    echo "unknown output mode: $output_mode (use table|json|csv)" >&2; exit 2 ;;
esac
