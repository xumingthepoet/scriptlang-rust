#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_JSON="$(mktemp)"
TMP_LOG="$(mktemp)"

cleanup() {
  rm -f "$TMP_JSON" "$TMP_LOG"
}
trap cleanup EXIT

cd "$ROOT_DIR"

if ! cargo llvm-cov \
  --workspace \
  --exclude sl-cli \
  --exclude sl-test-example \
  --all-features \
  --all-targets \
  --json \
  --output-path "$TMP_JSON" >"$TMP_LOG" 2>&1; then
  cat "$TMP_LOG"
  exit 1
fi

total_percent="$(jq -r '.data[0].totals.lines.percent' "$TMP_JSON")"
printf 'LINE_COVERAGE: %.2f%%\n' "$total_percent"

merge_ranges() {
  local csv="$1"
  local -a nums ranges
  IFS=',' read -r -a nums <<<"$csv"
  if [[ "${#nums[@]}" -eq 0 || -z "${nums[0]}" ]]; then
    echo ""
    return
  fi

  local start="${nums[0]}"
  local prev="${nums[0]}"
  local n
  for ((i = 1; i < ${#nums[@]}; i++)); do
    n="${nums[$i]}"
    if (( n == prev + 1 )); then
      prev="$n"
      continue
    fi
    if (( start == prev )); then
      ranges+=("$start")
    else
      ranges+=("$start-$prev")
    fi
    start="$n"
    prev="$n"
  done

  if (( start == prev )); then
    ranges+=("$start")
  else
    ranges+=("$start-$prev")
  fi
  (IFS=','; echo "${ranges[*]}")
}

jq -r '
  .data[0].files[]
  | .filename as $file
  | (
      [.segments[]
        | select(.[3] == true and .[5] == false)
        | {line: .[0], count: .[2]}
      ]
      | sort_by(.line)
      | group_by(.line)
      | map({line: .[0].line, count: (map(.count) | max)})
      | map(select(.count == 0) | .line)
    ) as $miss
  | select(($miss | length) > 0)
  | [$file, ($miss | length), ($miss | map(tostring) | join(","))]
  | @tsv
' "$TMP_JSON" | while IFS=$'\t' read -r file missing_count missing_csv; do
  rel_file="${file#"$ROOT_DIR"/}"
  merged="$(merge_ranges "$missing_csv")"
  echo "$rel_file: $missing_count uncovered lines [$merged]"
done

if ! awk "BEGIN { exit !($total_percent >= 100.0) }"; then
  echo "Coverage check failed: line coverage is below 100%."
  exit 1
fi
