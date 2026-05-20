#!/usr/bin/env bash
set -Eeu -o pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOCKET_NAME="tmux-thumbs-smoke-$$"
SESSION_NAME="thumbs-smoke"
TMUX_BIN="${TMUX_BIN:-tmux}"

tmux_cmd() {
  "${TMUX_BIN}" -L "${SOCKET_NAME}" "$@"
}

cleanup() {
  tmux_cmd kill-server >/dev/null 2>&1 || true
}

fail() {
  echo "smoke-tmux: $*" >&2
  exit 1
}

trap cleanup EXIT

cargo build --release --quiet

wait_for_window() {
  local name="$1"
  local deadline=$((SECONDS + 5))

  while [ "${SECONDS}" -lt "${deadline}" ]; do
    if tmux_cmd list-windows -F "#{window_name}" | grep -Fxq "${name}"; then
      return 0
    fi
    sleep 0.05
  done

  return 1
}

run_thumbs_and_exit() {
  local pane_id="$1"
  local socket_path
  local deadline

  tmux_cmd run-shell -b "${ROOT_DIR}/target/release/tmux-thumbs --dir '${ROOT_DIR}' --pane '${pane_id}'"
  wait_for_window "[thumbs]" || fail "thumbs window was not created"

  deadline=$((SECONDS + 5))
  while [ "${SECONDS}" -lt "${deadline}" ]; do
    socket_path="$(tmux_cmd list-keys -T thumbs 2>/dev/null | sed -n "s/.*--input-socket '\([^']*\)'.*/\1/p" | head -n 1)"
    if [ -n "${socket_path}" ] && [ -S "${socket_path}" ]; then
      break
    fi
    sleep 0.05
  done

  [ -n "${socket_path}" ] || fail "thumbs input socket was not created"

  "${ROOT_DIR}/target/release/tmux-thumbs" --input-socket "${socket_path}" --send-input esc
  sleep 0.4
}

assert_pane_alive() {
  local pane_id="$1"

  tmux_cmd display-message -p -t "${pane_id}" "#{pane_id}" >/dev/null 2>&1 || fail "pane ${pane_id} was closed"
}

assert_no_thumbs_windows() {
  if tmux_cmd list-windows -F "#{window_name}" | grep -Fxq "[thumbs]"; then
    fail "temporary [thumbs] window was left behind"
  fi
}

tmux_cmd new-session -d -s "${SESSION_NAME}" -x 100 -y 30 "printf 'https://example.com\n'; sleep 1000"
MAIN_PANE="$(tmux_cmd display-message -p -t "${SESSION_NAME}:0" "#{pane_id}")"
MAIN_WINDOW="$(tmux_cmd display-message -p -t "${MAIN_PANE}" "#{window_id}")"

run_thumbs_and_exit "${MAIN_PANE}" "${MAIN_WINDOW}"
assert_pane_alive "${MAIN_PANE}"
assert_no_thumbs_windows

SMALL_PANE="$(tmux_cmd split-window -P -F "#{pane_id}" -v -l 5 -t "${MAIN_PANE}" "printf 'https://zoom.example.com\n'; sleep 1000")"
tmux_cmd resize-pane -Z -t "${SMALL_PANE}"

run_thumbs_and_exit "${SMALL_PANE}" "${MAIN_WINDOW}"
assert_pane_alive "${SMALL_PANE}"
assert_no_thumbs_windows

echo "smoke-tmux: ok"
