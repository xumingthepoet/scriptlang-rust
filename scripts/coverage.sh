#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_LOG="$(mktemp)"
TMP_UNCOVERED="$(mktemp)"
TMP_OUT="$(mktemp)"

cleanup() {
  rm -f "$TMP_LOG" "$TMP_UNCOVERED" "$TMP_OUT"
}
trap cleanup EXIT

cd "$ROOT_DIR"

if ! cargo llvm-cov \
  --workspace \
  --exclude sl-cli \
  --exclude sl-test-example \
  --all-features \
  --all-targets \
  --show-missing-lines >"$TMP_LOG" 2>&1; then
  cat "$TMP_LOG"
  exit 1
fi

total_percent="$(
  awk '
    /TOTAL/ && /%/ {
      pct_idx = 0
      for (i = 1; i <= NF; i++) {
        if ($i ~ /^[0-9]+(\.[0-9]+)?%$/) {
          pct_idx++
          if (pct_idx != 3) {
            continue
          }
          gsub("%", "", $i)
          print $i
          exit
        }
      }
    }
  ' "$TMP_LOG"
)"

if [[ -z "${total_percent:-}" ]]; then
  echo "Failed to parse total line coverage from llvm-cov output."
  cat "$TMP_LOG"
  exit 1
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

normalize_ranges() {
  local raw="$1"
  local cleaned token start end n
  local -a parts nums
  cleaned="$(echo "$raw" | tr -d '[:space:]')"
  [[ -z "$cleaned" ]] && { echo ""; return; }
  IFS=',' read -r -a parts <<<"$cleaned"
  for token in "${parts[@]}"; do
    [[ -z "$token" ]] && continue
    if [[ "$token" == *-* ]]; then
      start="${token%-*}"
      end="${token#*-}"
      if [[ "$start" =~ ^[0-9]+$ && "$end" =~ ^[0-9]+$ && "$start" -le "$end" ]]; then
        for ((n = start; n <= end; n++)); do
          nums+=("$n")
        done
      fi
    elif [[ "$token" =~ ^[0-9]+$ ]]; then
      nums+=("$token")
    fi
  done
  if [[ "${#nums[@]}" -eq 0 ]]; then
    echo ""
    return
  fi
  printf '%s\n' "${nums[@]}" | sort -n | uniq | paste -sd, -
}

awk '
  BEGIN { in_uncovered = 0 }
  /^Uncovered Lines:/ { in_uncovered = 1; next }
  in_uncovered {
    if ($0 ~ /^[[:space:]]*$/) next
    if ($0 ~ /^[[:space:]]*[-=]+[[:space:]]*$/) next
    if (index($0, ":") == 0) next
    print
  }
' "$TMP_LOG" >"$TMP_UNCOVERED"

while IFS= read -r line; do
  [[ -z "$line" ]] && continue
  file="${line%%:*}"
  raw="${line#*:}"
  normalized="$(normalize_ranges "$raw")"
  [[ -z "$normalized" ]] && continue
  count="$(awk -F',' '{print NF}' <<<"$normalized")"
  merged="$(merge_ranges "$normalized")"
  rel_file="${file#"$ROOT_DIR"/}"
  printf '%s: %s uncovered lines [%s]\n' "$rel_file" "$count" "$merged" >>"$TMP_OUT"
done <"$TMP_UNCOVERED"

if [[ -s "$TMP_OUT" ]]; then
  sort "$TMP_OUT"
fi

effective_percent="$total_percent"
if [[ ! -s "$TMP_OUT" ]]; then
  effective_percent="100.00"
fi

if ! awk "BEGIN { exit !($effective_percent >= 100.0) }"; then
  echo "Coverage check failed: line coverage is below 100%."
  exit 1
fi
