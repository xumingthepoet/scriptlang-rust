#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

MAX_ROUNDS="${1:-10}"
PROMPT="利用improve-coverage skill来提高项目测试覆盖率。你在一个循环里工作：先专注解决一个 uncovered region，完成单点闭环后再看剩余上下文空间；如果上下文仍充足（20万 token），再继续处理下一个 region。不要在多个 region 之间来回切换，避免最后都没改好。每一轮至少要实际解决一个 region（通过新增/修改同文件测试来消除该 region 的未覆盖）。你有足够时间，优先稳定完成再扩展。"

if ! [[ "$MAX_ROUNDS" =~ ^[0-9]+$ ]] || [[ "$MAX_ROUNDS" -lt 1 ]]; then
  echo "Usage: $0 [rounds>=1]"
  exit 1
fi

for ((round = 1; round <= MAX_ROUNDS; round++)); do
  echo "===== Round ${round}/${MAX_ROUNDS}: invoking claude ====="
  claude -p --dangerously-skip-permissions "$PROMPT"

  echo "===== Round ${round}/${MAX_ROUNDS}: running make gate ====="
  set +e
  gate_output="$(make gate 2>&1)"
  gate_status=$?
  set -e
  echo "$gate_output"
  echo "make gate exit code: ${gate_status}"

  coverage_value="$(
    printf '%s\n' "$gate_output" \
      | sed -n 's/^REGION_COVERAGE:[[:space:]]*\([0-9][0-9]*\(\.[0-9]\+\)\?\)%$/\1/p' \
      | tail -n 1
  )"

  if [[ -n "$coverage_value" ]] && awk "BEGIN { exit !($coverage_value >= 100) }"; then
    echo "Coverage reached 100% (REGION_COVERAGE=${coverage_value}%), exiting loop."
    exit 0
  fi

  # If gate passed but coverage line wasn't parsed, stop to avoid blind looping.
  if [[ "$gate_status" -eq 0 ]]; then
    echo "make gate succeeded, exiting loop."
    exit 0
  fi

  if ! printf '%s\n' "$gate_output" | grep -q "uncovered regions"; then
    echo "No uncovered-region hints found, but make gate failed; continuing to next round."
    continue
  fi
done

echo "Reached max rounds (${MAX_ROUNDS}) without early exit."
