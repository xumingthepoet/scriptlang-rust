#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Stop process tree(s) rooted at claude-coverage-loop.sh.

Usage:
  scripts/stop-claude.sh [--dry-run]

Options:
  --dry-run  Print target PID tree list without stopping
  -h, --help Show this help message
USAGE
}

dry_run=0

for arg in "$@"; do
  case "$arg" in
    --dry-run)
      dry_run=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $arg" >&2
      usage >&2
      exit 1
      ;;
  esac
done

ps_table="$(ps -Ao pid=,ppid=)"
ps_table_with_cmd="$(ps -Ao pid=,ppid=,command=)"

roots_csv="$(
  awk -v self_pid="$$" '
    {
      pid = $1
      ppid = $2
      $1 = ""
      $2 = ""
      sub(/^ +/, "", $0)
      cmd = $0
      if (pid == self_pid) {
        next
      }
      if (cmd ~ /stop-claude\.sh/) {
        next
      }
      if (cmd ~ /claude-coverage-loop\.sh/) {
        if (out != "") {
          out = out "," pid
        } else {
          out = pid
        }
      }
    }
    END {
      if (out != "") {
        print out
      }
    }
  ' <<<"$ps_table_with_cmd"
)"

if [[ -z "$roots_csv" ]]; then
  echo "No running claude-coverage-loop.sh process found."
  exit 0
fi

echo "Found root claude-coverage-loop PID(s): ${roots_csv//,/ }"

declare -a targets=()
declare -a target_cmds=()

while IFS=$'\t' read -r pid cmd; do
  [[ -z "$pid" ]] && continue
  targets+=("$pid")
  target_cmds+=("$cmd")
done < <(
  awk -v roots_csv="$roots_csv" -v self_pid="$$" '
    {
      pid = $1
      ppid = $2
      $1 = ""
      $2 = ""
      sub(/^ +/, "", $0)
      cmd_by_pid[pid] = $0
      children[ppid] = children[ppid] " " pid
    }
    END {
      root_count = split(roots_csv, roots, ",")
      for (i = 1; i <= root_count; i++) {
        if (roots[i] != "") {
          queue[++queue_tail] = roots[i]
        }
      }

      for (i = 1; i <= queue_tail; i++) {
        current = queue[i]
        if (current == "" || seen[current]) {
          continue
        }
        if (current == self_pid) {
          continue
        }
        seen[current] = 1
        order[++order_count] = current

        child_count = split(children[current], child_list, " ")
        for (j = 1; j <= child_count; j++) {
          if (child_list[j] != "") {
            queue[++queue_tail] = child_list[j]
          }
        }
      }

      for (i = order_count; i >= 1; i--) {
        pid = order[i]
        print pid "\t" cmd_by_pid[pid]
      }
    }
  ' <<<"$ps_table_with_cmd"
)

if [[ "${#targets[@]}" -eq 0 ]]; then
  echo "No live target pid found."
  exit 0
fi

if [[ "$dry_run" -eq 1 ]]; then
  echo "Dry run. Would send SIGTERM in this order (leaf -> root):"
  for ((i = 0; i < ${#targets[@]}; i++)); do
    printf '  PID=%s CMD=%s\n' "${targets[$i]}" "${target_cmds[$i]}"
  done
  exit 0
fi

declare -a failed=()
for pid in "${targets[@]}"; do
  if ! kill "$pid" 2>/dev/null; then
    failed+=("$pid")
  fi
done

if [[ "${#failed[@]}" -gt 0 ]]; then
  echo "Failed to stop some pid(s): ${failed[*]}" >&2
  exit 1
fi

echo "Sent SIGTERM to PID tree(s): ${targets[*]}"
