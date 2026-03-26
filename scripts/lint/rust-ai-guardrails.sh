#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

if ! command -v ast-grep >/dev/null 2>&1; then
  echo "warning: ast-grep is not installed; skipping AI guardrail scan (install with: cargo install ast-grep --locked)" >&2
  exit 0
fi

BASE_REF="${AIGENRIV_BASE_REF:-}"
if [[ -z "$BASE_REF" ]]; then
  if git rev-parse --verify origin/main >/dev/null 2>&1; then
    BASE_REF="$(git merge-base origin/main HEAD)"
  else
    BASE_REF="HEAD~1"
  fi
fi

mapfile -t changed_rs < <(git diff --name-only --diff-filter=ACMR "$BASE_REF"...HEAD -- '*.rs' \
  | rg -v '(^|/)(tests?|testdata|fixtures)/|_test\.rs$|/target/' || true)

if [[ ${#changed_rs[@]} -eq 0 ]]; then
  echo "No changed Rust files to lint against AI guardrails."
  exit 0
fi

echo "Running ast-grep guardrails on ${#changed_rs[@]} changed Rust files..."
ast-grep scan --config .ast-grep/sgconfig.yml --error "${changed_rs[@]}"

echo "Rust AI guardrails passed."
