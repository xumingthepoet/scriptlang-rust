#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
START_TS="$(date +%s)"
TMP_LOG="$(mktemp)"
TMP_JSON="$(mktemp)"
TMP_UNCOVERED="$(mktemp)"
TMP_OUT="$(mktemp)"

cleanup() {
  rm -f "$TMP_LOG" "$TMP_JSON" "$TMP_UNCOVERED" "$TMP_OUT"
}

on_exit() {
  local status="$1"
  local end_ts elapsed
  end_ts="$(date +%s)"
  elapsed="$((end_ts - START_TS))"
  printf 'COVERAGE_TIME_USED_SECONDS: %ss\n' "$elapsed"
  cleanup
  exit "$status"
}
trap 'on_exit $?' EXIT

cd "$ROOT_DIR"

# Avoid stale profile/target artifacts causing mismatched coverage mapping.
cargo llvm-cov clean --workspace >/dev/null 2>&1 || true
rm -rf target/llvm-cov-target
if [[ -d target ]]; then
  find target -name "*.profraw" -delete
fi

if ! cargo llvm-cov \
  --workspace \
  --exclude sl-cli \
  --exclude sl-lint \
  --exclude sl-test-example \
  --all-features \
  --all-targets \
  --json \
  --output-path "$TMP_JSON" \
  -q >"$TMP_LOG" 2>&1; then
  cat "$TMP_LOG"
  exit 1
fi

total_percent="$(jq -r '.data[0].totals.regions.percent // empty' "$TMP_JSON")"
min_region_coverage="${MIN_REGION_COVERAGE:-99.50}"

if [[ -z "${total_percent:-}" ]]; then
  echo "Failed to parse total region coverage from llvm-cov output."
  cat "$TMP_LOG"
  exit 1
fi

printf 'REGION_COVERAGE: %.2f%%\n' "$total_percent"
printf 'MIN_REGION_COVERAGE: %.2f%%\n' "$min_region_coverage"

jq -r '
  .data[0].files[]
  | .filename as $f
  | [ .segments[]
      | select(.[2] == 0 and .[3] == true and .[4] == true)
      | "\(. [0]):\(. [1])"
    ] as $lines
  | if ($lines | length) > 0
    then "\($f)\t\($lines | join(","))"
    else empty
    end
' "$TMP_JSON" >"$TMP_UNCOVERED"

while IFS= read -r line; do
  [[ -z "$line" ]] && continue
  file="${line%%$'\t'*}"
  raw="${line#*$'\t'}"
  [[ -z "$raw" ]] && continue
  count="$(awk -F',' '{print NF}' <<<"$raw")"
  rel_file="${file#"$ROOT_DIR"/}"
  printf '%s: %s uncovered regions [%s]\n' "$rel_file" "$count" "$raw" >>"$TMP_OUT"
done <"$TMP_UNCOVERED"

if [[ -s "$TMP_OUT" ]]; then
  sort "$TMP_OUT"
fi

if ! awk "BEGIN { exit !($total_percent >= $min_region_coverage) }"; then
  echo "Coverage check failed: region coverage is below ${min_region_coverage}%."
  exit 1
fi
