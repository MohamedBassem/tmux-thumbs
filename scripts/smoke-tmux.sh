#!/usr/bin/env bash
set -Eeu -o pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOCKET_NAME="tmux-thumbs-smoke-$$"
SESSION_NAME="thumbs-smoke"
TMUX_BIN="${TMUX_BIN:-tmux}"
GIT_STATUS_REPO=""

tmux_cmd() {
  "${TMUX_BIN}" -L "${SOCKET_NAME}" "$@"
}

cleanup() {
  tmux_cmd kill-server >/dev/null 2>&1 || true
  if [ -n "${GIT_STATUS_REPO}" ]; then
    rm -rf "${GIT_STATUS_REPO}"
  fi
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

run_thumbs() {
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
  printf '%s\n' "${socket_path}"
}

run_thumbs_and_exit() {
  local pane_id="$1"
  local _window_id="$2"
  local socket_path

  socket_path="$(run_thumbs "${pane_id}")"
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

GIT_STATUS_REPO="$(mktemp -d "${TMPDIR:-/tmp}/tmux-thumbs-git-status.XXXXXX")"
mkdir -p "${GIT_STATUS_REPO}/.claude" "${GIT_STATUS_REPO}/tmux"
git -C "${GIT_STATUS_REPO}" init --quiet
git -C "${GIT_STATUS_REPO}" config user.email smoke@example.com
git -C "${GIT_STATUS_REPO}" config user.name Smoke
printf 'one\n' > "${GIT_STATUS_REPO}/.claude/settings.json"
printf 'one\n' > "${GIT_STATUS_REPO}/tmux/.tmux.conf"
git -C "${GIT_STATUS_REPO}" add .claude/settings.json tmux/.tmux.conf
git -C "${GIT_STATUS_REPO}" commit --quiet -m init
printf 'two\n' > "${GIT_STATUS_REPO}/.claude/settings.json"
printf 'two\n' > "${GIT_STATUS_REPO}/tmux/.tmux.conf"

STATUS_PANE="$(tmux_cmd new-window -P -F "#{pane_id}" -n git-status "cd '${GIT_STATUS_REPO}' && git status; sleep 1000")"
STATUS_WINDOW="$(tmux_cmd display-message -p -t "${STATUS_PANE}" "#{window_id}")"
tmux_cmd resize-window -t "${STATUS_WINDOW}" -x 70 -y 24
socket_path="$(run_thumbs "${STATUS_PANE}")"
screen="$(tmux_cmd capture-pane -p -t "${STATUS_WINDOW}")"
status_fail() {
  printf '%s\n' "${screen}" >&2
  fail "$1"
}

printf '%s\n' "${screen}" | grep -Fq "modified:   .claude/settings.json" || status_fail "git status settings marker was misplaced"
printf '%s\n' "${screen}" | grep -Fq "modified:   tmux/.tmux.conf" || status_fail "git status tmux marker was misplaced"
if printf '%s\n' "${screen}" | grep -Eq "modifi[^:]*\\.claude|modifi[^:]*tmux|settings\\.jsons\\.json|\\.tmux\\.confx\\.conf"; then
  status_fail "git status overlay corrupted file marker text"
fi

"${ROOT_DIR}/target/release/tmux-thumbs" --input-socket "${socket_path}" --send-input esc
sleep 0.4
assert_pane_alive "${STATUS_PANE}"
assert_no_thumbs_windows

echo "smoke-tmux: ok"
