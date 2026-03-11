#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

MAX_ROUNDS="${1:-10}"
PROMPT_ROUND_1="利用improve-coverage skill来提高项目测试覆盖率。你在一个循环里工作：先专注解决一个 uncovered region，完成单点闭环后再看剩余上下文空间；如果上下文仍充足（20万 token），再继续处理下一个 region。不要在多个 region 之间来回切换，避免最后都没改好。每一轮至少要实际解决一个 region（通过新增/修改同文件测试来消除该 region 的未覆盖）；在已完成至少一个 region 的前提下，若 token 空间仍充足，可以继续处理下一个 region。若定位到死分支/不可达代码，允许修改源码并删除不可达代码，不要为了覆盖率硬写伪测试命中该路径。"
PROMPT_ROUND_2="继续推进覆盖率任务，不要提问“是否继续”。请先基于当前仓库状态自行定位一个最高收益且低风险的 uncovered region，完成一个单点闭环（仅处理一个 region，含改动与最小验证），并汇报本次实际消除的 region。"
PROMPT_ROUND_3="继续推进覆盖率任务，不要反问。请重新评估当前仓库后再处理一个新的 uncovered region；要求小步快跑、单点闭环、避免跨多个 region 来回切换。若当前没有可安全推进项，输出“STOP_ROUND”并给出阻塞原因。"

PROMPTS=(
  "$PROMPT_ROUND_1"
  "$PROMPT_ROUND_2"
  "$PROMPT_ROUND_3"
)

if ! [[ "$MAX_ROUNDS" =~ ^[0-9]+$ ]] || [[ "$MAX_ROUNDS" -lt 1 ]]; then
  echo "Usage: $0 [rounds>=1]"
  exit 1
fi

for ((round = 1; round <= MAX_ROUNDS; round++)); do
  echo "===== Round ${round}/${MAX_ROUNDS}: invoking claude (${#PROMPTS[@]} prompts) ====="
  claude_failed=0
  for ((i = 0; i < ${#PROMPTS[@]}; i++)); do
    prompt_idx=$((i + 1))
    prompt="${PROMPTS[$i]}"
    if [[ "$i" -eq 0 ]]; then
      claude_cmd=(claude -p --dangerously-skip-permissions "$prompt")
    else
      claude_cmd=(claude -p -c --dangerously-skip-permissions "$prompt")
    fi

    echo "----- Round ${round}: prompt ${prompt_idx}/${#PROMPTS[@]} -----"
    set +e
    claude_output="$("${claude_cmd[@]}" 2>&1)"
    claude_status=$?
    set -e
    echo "$claude_output"
    echo "claude exit code: ${claude_status}"
    if [[ "$claude_status" -ne 0 ]]; then
      claude_failed=1
      break
    fi
  done

  if [[ "$claude_failed" -ne 0 ]]; then
    echo "claude failed in round ${round}; sleeping 10 minutes and skipping make gate for this round."
    sleep 600
    continue
  fi

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

  # Even when gate succeeds, keep looping until MAX_ROUNDS.
  if [[ "$gate_status" -eq 0 ]]; then
    echo "make gate succeeded; continuing to next round."
    continue
  fi

  if ! printf '%s\n' "$gate_output" | grep -q "uncovered regions"; then
    echo "No uncovered-region hints found, but make gate failed; continuing to next round."
    continue
  fi
done

echo "Reached max rounds (${MAX_ROUNDS}) without early exit."
