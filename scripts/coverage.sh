#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_LCOV="$(mktemp)"
TMP_LOG="$(mktemp)"

cleanup() {
  rm -f "$TMP_LCOV" "$TMP_LOG"
}
trap cleanup EXIT

cd "$ROOT_DIR"

if ! cargo llvm-cov \
  --workspace \
  --exclude sl-cli \
  --exclude sl-test-example \
  --all-features \
  --all-targets \
  --lcov \
  --output-path "$TMP_LCOV" >"$TMP_LOG" 2>&1; then
  cat "$TMP_LOG"
  exit 1
fi

read -r covered_lines total_lines < <(
  awk -F: '
    /^LH:/ { covered += $2 }
    /^LF:/ { total += $2 }
    END { printf "%d %d\n", covered + 0, total + 0 }
  ' "$TMP_LCOV"
)
if [[ "$total_lines" -eq 0 ]]; then
  total_percent="100.00"
else
  total_percent="$(awk -v c="$covered_lines" -v t="$total_lines" 'BEGIN { printf "%.2f", (c * 100.0) / t }')"
fi
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

awk -F: '
  /^SF:/ { file = substr($0, 4); next }
  /^DA:/ {
    split($2, parts, ",")
    print file "\t" (parts[1] + 0) "\t" (parts[2] + 0)
    next
  }
' "$TMP_LCOV" \
  | sort -t $'\t' -k1,1 -k2,2n \
  | awk -F'\t' '
    function flush_file() {
      if (current_file != "" && miss_count > 0) {
        print current_file "\t" miss_count "\t" miss_csv
      }
    }
    function flush_line() {
      if (current_file != "" && current_line >= 0 && line_has_coverage == 0) {
        miss_count++
        miss_csv = miss_csv ((miss_csv == "") ? "" : ",") current_line
      }
    }
    {
      file = $1
      line = $2 + 0
      count = $3 + 0

      if (current_file != file) {
        flush_line()
        flush_file()
        current_file = file
        current_line = -1
        line_has_coverage = 0
        miss_count = 0
        miss_csv = ""
      }

      if (current_line != line) {
        flush_line()
        current_line = line
        line_has_coverage = 0
      }
      if (count > 0) {
        line_has_coverage = 1
      }
    }
    END {
      flush_line()
      flush_file()
    }
  ' | while IFS=$'\t' read -r file missing_count missing_csv; do
  rel_file="${file#"$ROOT_DIR"/}"
  merged="$(merge_ranges "$missing_csv")"
  echo "$rel_file: $missing_count uncovered lines [$merged]"
done

if ! awk "BEGIN { exit !($total_percent >= 100.0) }"; then
  echo "Coverage check failed: line coverage is below 100%."
  exit 1
fi
