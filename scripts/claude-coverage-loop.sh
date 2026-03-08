#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

MAX_ROUNDS="${1:-10}"
PROMPT="利用improve-coverage skill来提高项目测试覆盖率"

if ! [[ "$MAX_ROUNDS" =~ ^[0-9]+$ ]] || [[ "$MAX_ROUNDS" -lt 1 ]]; then
  echo "Usage: $0 [rounds>=1]"
  exit 1
fi

for ((round = 1; round <= MAX_ROUNDS; round++)); do
  echo "===== Round ${round}/${MAX_ROUNDS}: invoking claude ====="
  claude -p --dangerously-skip-permissions "$PROMPT"

  echo "===== Round ${round}/${MAX_ROUNDS}: running make gate ====="
  gate_output="$(make gate 2>&1)"
  echo "$gate_output"

  coverage_value="$(
    printf '%s\n' "$gate_output" \
      | sed -n 's/^REGION_COVERAGE:[[:space:]]*\([0-9][0-9]*\(\.[0-9]\+\)\?\)%$/\1/p' \
      | tail -n 1
  )"

  if [[ -n "$coverage_value" ]] && awk "BEGIN { exit !($coverage_value >= 100) }"; then
    echo "Coverage reached 100% (REGION_COVERAGE=${coverage_value}%), exiting loop."
    exit 0
  fi

  if ! printf '%s\n' "$gate_output" | grep -q "uncovered regions"; then
    echo "No uncovered-region hints found, exiting loop."
    exit 0
  fi
done

echo "Reached max rounds (${MAX_ROUNDS}) without early exit."
