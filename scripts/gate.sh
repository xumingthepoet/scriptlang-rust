#!/usr/bin/env bash
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${1:-gate}"

now_seconds() {
  date +%s
}

print_ok() {
  local name="$1"
  local elapsed="$2"
  printf '[OK]   %s (%ss)\n' "$name" "$elapsed"
}

print_fail() {
  local name="$1"
  local elapsed="$2"
  printf '[FAIL] %s (%ss)\n' "$name" "$elapsed"
}

run_step() {
  local name="$1"
  shift

  printf '[RUN]  %s\n' "$name"
  local start
  start="$(now_seconds)"
  local log_file
  log_file="$(mktemp)"

  if "$@" >"$log_file" 2>&1; then
    local end
    end="$(now_seconds)"
    print_ok "$name" "$((end - start))"
    rm -f "$log_file"
    return 0
  fi

  local end
  end="$(now_seconds)"
  print_fail "$name" "$((end - start))"
  cat "$log_file"
  rm -f "$log_file"
  return 1
}

require_tool() {
  local tool="$1"
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Missing required tool: $tool"
    return 1
  fi
}

print_coverage_summary() {
  local report_json="$1"

  local global
  global="$(jq -r '"GLOBAL covered=\(.covered) coverable=\(.coverable) coverage=\(.coverage)%"' "$report_json")"
  echo "$global"
  echo "CRATE COVERAGE"
  jq -r '
    .files
    | map({
        crate: (
          if (.path | index("crates")) != null
          then .path[(.path | index("crates")) + 1]
          else "workspace-root"
          end
        ),
        coverable: (.traces | length),
        covered: (.traces | map(select((.stats.Line // 0) > 0)) | length)
      })
    | sort_by(.crate)
    | group_by(.crate)
    | map({
        crate: .[0].crate,
        covered: (map(.covered) | add),
        coverable: (map(.coverable) | add)
      })
    | .[]
    | . + {coverage: (if .coverable == 0 then 0 else ((.covered / .coverable) * 100) end)}
    | [.crate, .covered, .coverable, .coverage]
    | @tsv
  ' "$report_json" | while IFS=$'\t' read -r crate covered coverable coverage; do
    printf '  %-16s %6s/%-6s %7.2f%%\n' "$crate" "$covered" "$coverable" "$coverage"
  done
}

run_coverage() {
  require_tool jq || return 1

  printf '[RUN]  coverage\n'
  local start
  start="$(now_seconds)"

  local log_file tmp_dir report_json
  log_file="$(mktemp)"
  tmp_dir="$(mktemp -d)"
  report_json="$tmp_dir/tarpaulin-report.json"

  if cargo tarpaulin \
    --engine llvm \
    --workspace \
    --all-features \
    --all-targets \
    --rustflags=--cfg=coverage \
    --out Json \
    --output-dir "$tmp_dir" \
    --fail-under 100 >"$log_file" 2>&1; then
    local end
    end="$(now_seconds)"
    print_ok "coverage" "$((end - start))"
    print_coverage_summary "$report_json"
    rm -f "$log_file"
    rm -rf "$tmp_dir"
    return 0
  fi

  local end
  end="$(now_seconds)"
  print_fail "coverage" "$((end - start))"
  cat "$log_file"
  rm -f "$log_file"
  rm -rf "$tmp_dir"
  return 1
}

run_gate() {
  local start
  start="$(now_seconds)"

  run_step "check" cargo qk || return 1
  run_step "fmt" cargo qa || return 1
  run_step "lint" cargo qc || return 1
  run_step "test" cargo qt || return 1
  run_coverage || return 1

  local end
  end="$(now_seconds)"
  printf '[DONE] gate (%ss)\n' "$((end - start))"
}

cd "$ROOT_DIR"
case "$TARGET" in
  gate)
    run_gate
    ;;
  coverage)
    run_coverage
    ;;
  check)
    run_step "check" cargo qk
    ;;
  fmt)
    run_step "fmt" cargo qa
    ;;
  lint)
    run_step "lint" cargo qc
    ;;
  test)
    run_step "test" cargo qt
    ;;
  *)
    echo "Unknown target: $TARGET"
    echo "Usage: bash scripts/gate.sh [gate|check|fmt|lint|test|coverage]"
    exit 2
    ;;
esac
