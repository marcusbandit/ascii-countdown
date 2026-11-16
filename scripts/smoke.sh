#!/usr/bin/env bash
# The checks are strings run by eval, so single quotes are on purpose.
# shellcheck disable=SC2016
# End-to-end check: runs tock in a headless tmux, presses keys, and checks what
# the screen shows, the exit codes, and that the terminal is put back.
#
#   scripts/smoke.sh [path/to/tock]
set -euo pipefail
cd "$(dirname "$0")/.."

bin=$(realpath "${1:-target/release/tock}")
sock=tock-smoke-$$
work=$(mktemp -d)
t() { tmux -L "$sock" -f /dev/null "$@"; }
trap 't kill-server 2>/dev/null || true; rm -rf "$work"' EXIT

fails=0
pass() { printf 'ok    %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1"; fails=$((fails + 1)); }
check() { if eval "$2"; then pass "$1"; else fail "$1"; fi; }

# start NAME ARGS...: runs tock in its own window; afterwards the shell prints
# the exit code and stays, so the restored screen can be inspected.
start() {
  local name=$1
  shift
  t new-window -d -n "$name" -t smoke: \
    "$bin $* > $work/$name.out; echo \"EXIT=\$?\"; exec cat"
  sleep 0.6
}
screen() { t capture-pane -p -t "smoke:$1"; }
flags() { t display-message -p -t "smoke:$1" '#{alternate_on} #{cursor_flag}'; }
exit_code() { screen "$1" | sed -n 's/^EXIT=//p'; }

t new-session -d -s smoke -x 100 -y 26 -n idle
t set -g status off
t set -g remain-on-exit on

start cd -t Tea 3m
check "countdown takes over the screen and hides the cursor" '[[ $(flags cd) == "1 0" ]]'
check "countdown shows its title" 'screen cd | grep -q "Tea"'
check "countdown draws big digits" '[[ $(screen cd | grep -c "█") -ge 5 ]]'
t send-keys -t smoke:cd Space
sleep 0.3
check "space pauses" 'screen cd | grep -q PAUSED'
t send-keys -t smoke:cd r
sleep 0.3
check "r resets and keeps it paused" 'screen cd | grep -q PAUSED'
t send-keys -t smoke:cd q
sleep 0.4
check "q leaves the alternate screen and shows the cursor" '[[ $(flags cd) == "0 1" ]]'
check "quitting a countdown early exits 1" '[[ $(exit_code cd) == 1 ]]'

start fin 1s
sleep 1.2
check "zero shows TIME'S UP" "screen fin | grep -q \"TIME'S UP\""
# tock ignores keys for 600 ms after zero; wait well past that.
sleep 1
t send-keys -t smoke:fin x
sleep 0.4
check "any key quits a finished countdown with 0" '[[ $(exit_code fin) == 0 ]]'

start sw stopwatch
sleep 1.2
check "stopwatch counts up" '! screen sw | grep -q PAUSED'
t send-keys -t smoke:sw q
sleep 0.4
check "stopwatch quits with 0" '[[ $(exit_code sw) == 0 ]]'
check "stopwatch prints the time it stopped at" 'grep -Eq "^00:0[1-2]\.[0-9]$" "$work/sw.out"'

start ctrlc --up
t send-keys -t smoke:ctrlc C-c
sleep 0.4
check "Ctrl+C restores the terminal" '[[ $(flags ctrlc) == "0 1" ]]'
check "Ctrl+C exits 130" '[[ $(exit_code ctrlc) == 130 ]]'

start term 10m
pkill -TERM -f "^$bin 10m" || true
sleep 0.4
check "SIGTERM restores the terminal" '[[ $(flags term) == "0 1" ]]'
check "SIGTERM exits 130" '[[ $(exit_code term) == 130 ]]'

start small 25m
t resize-window -t smoke:small -x 30 -y 8
sleep 0.4
check "a resize to a small window still shows the time" 'screen small | grep -q "25:00\|24:59\|█"'
t send-keys -t smoke:small q

# shellcheck disable=SC2034 # read by the eval in check
"$bin" 5x 2>/dev/null && rc=0 || rc=$?
check "a bad duration exits 2" '[[ $rc == 2 ]]'

if ((fails)); then
  echo "$fails check(s) failed"
  exit 1
fi
echo "all checks passed"
