#!/usr/bin/env bash
# Toggle subagent permission-related settings for manual testing.
# Usage: ./scripts/subagent-permission/setup.sh <defaults|deny|escalate|writable-explore|show>
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

BIN=(cargo run --quiet --)
if [[ -x "$ROOT/target/release/wgenty-code" ]]; then
  BIN=("$ROOT/target/release/wgenty-code")
elif [[ -x "$ROOT/target/debug/wgenty-code" ]]; then
  BIN=("$ROOT/target/debug/wgenty-code")
fi

cfg() {
  "${BIN[@]}" config set "$1" "$2"
}

show_relevant() {
  echo "== relevant settings (from config show | filter) =="
  "${BIN[@]}" config show 2>/dev/null | rg -n "ask_strategy|explore_readonly|approval_timeout|timeout_decision|permission_mode|subagent" || true
  echo
  echo "settings file: ${HOME}/.wgenty-code/settings.json"
}

mode="${1:-}"
case "$mode" in
  defaults)
    cfg agent.subagent.ask_strategy escalate_to_user
    cfg agent.subagent.explore_readonly true
    cfg agent.subagent.approval_timeout_secs 60
    cfg agent.subagent.timeout_decision deny
    echo "OK: defaults restored (explore_readonly=true, ask_strategy=escalate_to_user, timeout=60)"
    ;;
  deny)
    cfg agent.subagent.ask_strategy deny
    cfg agent.subagent.explore_readonly true
    cfg agent.subagent.approval_timeout_secs 60
    echo "OK: ask_strategy=deny (no approval UI for policy Ask)"
    ;;
  escalate)
    cfg agent.subagent.ask_strategy escalate_to_user
    cfg agent.subagent.explore_readonly true
    cfg agent.subagent.approval_timeout_secs 15
    cfg agent.subagent.timeout_decision deny
    echo "OK: escalate_to_user + approval_timeout_secs=15 (good for timeout test)"
    ;;
  writable-explore)
    cfg agent.subagent.explore_readonly false
    cfg agent.subagent.ask_strategy escalate_to_user
    echo "OK: explore_readonly=false (explore/plan keep mutating FS tools in allowlist)"
    ;;
  show)
    show_relevant
    exit 0
    ;;
  *)
    cat <<'EOF'
Usage: setup.sh <command>

  defaults           restore safe defaults
  deny               ask_strategy=deny
  escalate           escalate_to_user + 15s approval timeout
  writable-explore   explore_readonly=false
  show               print relevant settings

Restart REPL after changing settings.
EOF
    exit 1
    ;;
esac

show_relevant
echo
echo "Restart REPL so settings reload: cargo run -- repl"
