#!/usr/bin/env bash
# Overnight eval — ingest hstry sessions into one mmry store per ingest config,
# run the recall eval against each, and produce a side-by-side comparison so
# you can leave this running overnight and pick the winning config in the morning.
#
# Usage:
#   ./scripts/run-overnight-eval.sh
#   QUERIES=eval/queries.json ./scripts/run-overnight-eval.sh
#   OUTDIR=eval/runs/2026-05-17-overnight ./scripts/run-overnight-eval.sh   # resume an existing run
#   INGEST_LIMIT=2000 SEARCH_LIMIT=20 ./scripts/run-overnight-eval.sh
#   CONFIGS='quick:quick:0,full-8k:full:8000' ./scripts/run-overnight-eval.sh
#
# Resumable: any config whose results.json already exists in OUTDIR is skipped.
# To force a re-run of a single config, delete its results.json under OUTDIR/configs/<name>/.
#
# Default configs (override via CONFIGS, format "name:mode:max_chars[,...]"):
#   quick     title + first user message (no per-session hstry show)
#   full-2k   title + transcripts truncated to 2 KB per session
#   full-8k   title + transcripts truncated to 8 KB per session
#   full-20k  title + transcripts truncated to 20 KB per session

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
QUERIES="${QUERIES:-$ROOT/eval/queries.json}"
INGEST_LIMIT="${INGEST_LIMIT:-2000}"
SEARCH_LIMIT="${SEARCH_LIMIT:-20}"
K_VALUES="${K_VALUES:-1,3,5,10}"
ENGINES="${ENGINES:-mmry-hybrid,mmry-hybrid-norerank,mmry-semantic,mmry-bm25,mmry-sparse,mmry-keyword,mmry-fuzzy,hstry-search}"
CONFIGS="${CONFIGS:-quick:quick:0,full-2k:full:2000,full-8k:full:8000,full-20k:full:20000}"

TS="$(date +%Y%m%d-%H%M%S)"
OUTDIR="${OUTDIR:-$ROOT/eval/runs/$TS}"
LOG="$OUTDIR/run.log"

mkdir -p "$OUTDIR/configs"

log() { printf '[%s] %s\n' "$(date +%H:%M:%S)" "$*" | tee -a "$LOG" >&2; }
die() { log "FATAL: $*"; exit 1; }

# ── prereqs ───────────────────────────────────────────────────────────────────

command -v mmry  >/dev/null || die "mmry not in PATH"
command -v hstry >/dev/null || die "hstry not in PATH"
command -v jq    >/dev/null || die "jq not in PATH"
[[ -f "$QUERIES" ]] || die "queries file not found: $QUERIES"

# Verify the mmry service is reachable up front — otherwise we'd waste hours
# discovering it later.
if ! mmry stores list >/dev/null 2>&1; then
  die "mmry CLI errored on 'stores list' — is the service running?"
fi

INGEST="$ROOT/scripts/test-mmry-session-ingest.sh"
EVAL="$ROOT/scripts/eval-mmry-recall.sh"
[[ -x "$INGEST" ]] || die "ingest script not executable: $INGEST"
[[ -x "$EVAL"   ]] || die "eval script not executable:   $EVAL"

QCOUNT=$(jq '.queries | length' "$QUERIES")

log "Overnight eval start"
log "  outdir       = $OUTDIR"
log "  queries      = $QUERIES ($QCOUNT items)"
log "  ingest_limit = $INGEST_LIMIT   search_limit = $SEARCH_LIMIT   K = $K_VALUES"
log "  engines      = $ENGINES"
log "  configs      = $CONFIGS"

IFS=',' read -ra CFG_LIST <<< "$CONFIGS"

# ── per-config runner ────────────────────────────────────────────────────────

drop_store_if_exists() {
  local store="$1"
  if mmry stores list 2>/dev/null | grep -wq "$store"; then
    log "  dropping existing store: $store"
    mmry stores delete "$store" --yes      >/dev/null 2>&1 \
      || mmry stores delete "$store" --force >/dev/null 2>&1 \
      || mmry stores delete "$store"          >/dev/null 2>&1 \
      || log "  WARN: could not delete $store (continuing — create may fail)"
  fi
}

run_config() {
  local cfg="$1"
  local name="${cfg%%:*}"
  local rest="${cfg#*:}"
  local mode="${rest%%:*}"
  local max_chars="${rest#*:}"
  local store="hstry-eval-${name}"
  local cdir="$OUTDIR/configs/$name"
  mkdir -p "$cdir"

  if [[ -s "$cdir/results.json" ]]; then
    log "[$name] resume: results.json present — skipping"
    return 0
  fi

  log "[$name] start  mode=$mode  max_chars=$max_chars  store=$store"
  local t_start
  t_start=$(date +%s)

  drop_store_if_exists "$store"
  mmry stores create "$store" >/dev/null 2>&1 \
    || { log "[$name] FAIL: could not create store"; return 1; }

  # ingest
  log "[$name] ingesting (tail $cdir/ingest.log to watch)…"
  STORE="$store" LIMIT="$INGEST_LIMIT" MODE="$mode" MAX_CHARS="$max_chars" \
    "$INGEST" >"$cdir/ingest.log" 2>&1
  local ic=$?
  if [[ $ic -ne 0 ]]; then
    log "[$name] FAIL: ingest exited $ic — see $cdir/ingest.log"
    return 1
  fi
  local ingested
  ingested=$(grep -c '^  -> ' "$cdir/ingest.log" || true)
  log "[$name] ingested ~$ingested sessions"

  # eval → json
  log "[$name] eval…"
  STORE="$store" LIMIT="$SEARCH_LIMIT" K_VALUES="$K_VALUES" ENGINES="$ENGINES" \
    "$EVAL" "$QUERIES" json >"$cdir/results.json" 2>"$cdir/eval.log"
  local ec=$?
  if [[ $ec -ne 0 ]]; then
    log "[$name] FAIL: eval exited $ec — see $cdir/eval.log"
    rm -f "$cdir/results.json"
    return 1
  fi

  # CSV for spreadsheet inspection
  STORE="$store" LIMIT="$SEARCH_LIMIT" K_VALUES="$K_VALUES" ENGINES="$ENGINES" \
    "$EVAL" "$QUERIES" csv >"$cdir/results.csv" 2>>"$cdir/eval.log" || true

  local dt=$(( $(date +%s) - t_start ))
  log "[$name] done in ${dt}s — $cdir/results.json"
}

# ── run all configs (failures don't abort the rest) ──────────────────────────

for cfg in "${CFG_LIST[@]}"; do
  run_config "$cfg" || log "WARN: $cfg did not complete cleanly"
done

# ── master summary ───────────────────────────────────────────────────────────

log "Aggregating master summary"
SUMMARY="$OUTDIR/summary.md"

{
  echo "# Overnight eval — $TS"
  echo
  echo "- queries: \`$(realpath --relative-to="$ROOT" "$QUERIES" 2>/dev/null || echo "$QUERIES")\` ($QCOUNT items)"
  echo "- ingest_limit: $INGEST_LIMIT  search_limit: $SEARCH_LIMIT  K: $K_VALUES"
  echo "- engines: \`$ENGINES\`"
  echo
  echo "## Master ranking (sorted by MRR, descending)"
  echo
  echo "| config | engine | N | MRR | hit@1 | hit@3 | hit@5 | hit@10 | mean_ms |"
  echo "| --- | --- | --- | --- | --- | --- | --- | --- | --- |"
} >"$SUMMARY"

ROWS="$OUTDIR/_rows.tsv"
: >"$ROWS"
for cfg in "${CFG_LIST[@]}"; do
  name="${cfg%%:*}"
  rj="$OUTDIR/configs/$name/results.json"
  [[ -s "$rj" ]] || continue
  jq -r --arg cfg "$name" '
    .summary[] | [
      $cfg, .engine, .n, .mrr,
      (.hits["hit@1"]  // 0),
      (.hits["hit@3"]  // 0),
      (.hits["hit@5"]  // 0),
      (.hits["hit@10"] // 0),
      .mean_lat_ms
    ] | @tsv
  ' "$rj" >>"$ROWS"
done

sort -k4,4 -gr "$ROWS" | awk -F'\t' '{
  printf("| %s | %s | %s | %.3f | %.2f | %.2f | %.2f | %.2f | %s |\n",
    $1, $2, $3, $4, $5, $6, $7, $8, $9)
}' >>"$SUMMARY"

{
  echo
  echo "## Per-config detail"
  for cfg in "${CFG_LIST[@]}"; do
    name="${cfg%%:*}"
    rj="$OUTDIR/configs/$name/results.json"
    echo
    if [[ ! -s "$rj" ]]; then
      echo "### $name — **missing** (config did not produce results)"
      continue
    fi
    echo "### $name"
    echo
    echo "| engine | N | MRR | hit@1 | hit@3 | hit@5 | hit@10 | mean_ms |"
    echo "| --- | --- | --- | --- | --- | --- | --- | --- |"
    jq -r '.summary | sort_by(-.mrr)[] | [
      .engine, .n, .mrr,
      (.hits["hit@1"]  // 0),
      (.hits["hit@3"]  // 0),
      (.hits["hit@5"]  // 0),
      (.hits["hit@10"] // 0),
      .mean_lat_ms
    ] | @tsv' "$rj" | awk -F'\t' '{
      printf("| %s | %s | %.3f | %.2f | %.2f | %.2f | %.2f | %s |\n",
        $1, $2, $3, $4, $5, $6, $7, $8)
    }'
  done

  echo
  echo "## Per-query rank by config (lower is better; 0 = miss)"
  echo
  echo "For each query and engine, the best-ranked hit across configs."
  echo
} >>"$SUMMARY"

# Per-query breakdown: pick best (lowest non-zero rank) engine per config and per query.
{
  echo "| query | best engine | best config | best rank |"
  echo "| --- | --- | --- | --- |"
  python3 - "$OUTDIR" "${CFG_LIST[@]}" 2>/dev/null <<'PY' || true
import json, os, sys
outdir = sys.argv[1]
cfgs = [c.split(':')[0] for c in sys.argv[2:]]
rows = {}  # query_name -> list of (rank, engine, cfg)
for cfg in cfgs:
    p = os.path.join(outdir, "configs", cfg, "results.json")
    if not os.path.exists(p): continue
    with open(p) as f:
        data = json.load(f)
    for q in data.get("queries", []):
        for r in q.get("results", []):
            rank = r.get("rank", 0) or 0
            if rank == 0: continue
            rows.setdefault(q["name"], []).append((rank, r["engine"], cfg))
for qname, hits in sorted(rows.items()):
    hits.sort()
    rank, eng, cfg = hits[0]
    print(f"| {qname} | {eng} | {cfg} | {rank} |")
# also list missing queries (all engines/configs missed)
all_qs = set()
for cfg in cfgs:
    p = os.path.join(outdir, "configs", cfg, "results.json")
    if not os.path.exists(p): continue
    with open(p) as f:
        data = json.load(f)
    for q in data.get("queries", []):
        all_qs.add(q["name"])
for qname in sorted(all_qs - set(rows.keys())):
    print(f"| {qname} | — | — | 0 |")
PY
} >>"$SUMMARY"

rm -f "$ROWS"

log "DONE — full summary at $SUMMARY"
echo
echo "════════════════════════════════════════════════════════════════════════"
cat "$SUMMARY"
echo "════════════════════════════════════════════════════════════════════════"
echo
echo "Artifacts:"
echo "  summary:  $SUMMARY"
echo "  configs:  $OUTDIR/configs/"
echo "  run log:  $LOG"
