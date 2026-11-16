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
t send-keys -t smoke:fin z
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

count() { screen "$1" | grep -o "$2" | wc -l; }

start multi -t Tea 3m -t Run stopwatch
check "several clocks show their titles" 'screen multi | grep -q "Tea" && screen multi | grep -q "Run"'
check "the selection frame starts hidden" '! screen multi | grep -q "╭"'
check "with nothing selected the bar only says esc" 'screen multi | grep -q "esc list" && ! screen multi | grep -q "space"'
t send-keys -t smoke:multi l
sleep 0.3
check "l moves to the next clock and shows the frame" 'screen multi | grep -q "╭"'
check "a selected clock shows its keys" 'screen multi | grep -q "space pause" && screen multi | grep -q "esc list"'
t send-keys -t smoke:multi Space
sleep 0.3
check "space pauses only the selected clock" '[[ $(count multi PAUSED) == 1 ]]'
t send-keys -t smoke:multi P
sleep 0.3
check "P then pauses every clock" '[[ $(count multi PAUSED) == 2 ]]'
t send-keys -t smoke:multi P
sleep 0.3
check "P again resumes every clock" '[[ $(count multi PAUSED) == 0 ]]'
t send-keys -t smoke:multi q
sleep 0.4
check "quitting with a countdown left exits 1" '[[ $(exit_code multi) == 1 ]]'
check "the stopwatch time still prints" '[[ $(wc -l < "$work/multi.out") == 1 ]]'

start rnd -t Laps rounds
sleep 0.4
t send-keys -t smoke:rnd n
sleep 0.3
check "n starts the next round" 'screen rnd | grep -q "ROUND 2   AVG"'
t send-keys -t smoke:rnd q
sleep 0.4
check "a rounds clock prints its total" 'grep -Eq "^00:0[0-9]\.[0-9]$" "$work/rnd.out"'

start pair 1s 1s
sleep 2.2
check "two finished clocks both show TIME'S UP" "[[ \$(count pair \"TIME'S UP\") == 2 ]]"
t send-keys -t smoke:pair z
sleep 0.4
check "any key quits once every clock finished" '[[ $(exit_code pair) == 0 ]]'

start again 1s 1s
sleep 2.2
t send-keys -t smoke:again R
sleep 0.4
check "R starts finished clocks over instead of quitting" "[[ -z \$(exit_code again) ]] && [[ \$(count again \"TIME'S UP\") == 0 ]]"
t send-keys -t smoke:again q

start mixed 1s 5m
sleep 2.2
t send-keys -t smoke:mixed z
sleep 0.4
check "a key does not quit while another clock runs" '[[ -z $(exit_code mixed) ]]'
t send-keys -t smoke:mixed q
sleep 0.4
check "q quits it with 1" '[[ $(exit_code mixed) == 1 ]]'

start list
check "no arguments opens the clock list" 'screen list | grep -q "No clocks"'
t send-keys -t smoke:list a
sleep 0.2
t send-keys -t smoke:list -l Tea
check "a new clock starts as a stopwatch" 'screen list | grep -q "› Name" && ! screen list | grep -q "Time"'
check "the form previews it in big digits" '[[ $(screen list | grep -c "█") -ge 5 ]]'
# Up to the type, over to countdown, then down past the name to the time.
t send-keys -t smoke:list BTab Right Tab Tab
t send-keys -t smoke:list -l 2m
t send-keys -t smoke:list Enter
sleep 0.3
check "a adds a clock through the form" 'screen list | grep -q "Tea" && screen list | grep -Eq "0[12]:[0-9]{2}"'
t send-keys -t smoke:list Enter
sleep 0.3
check "enter shows it in big digits" '[[ $(screen list | grep -c "█") -ge 5 ]]'
t send-keys -t smoke:list Escape
sleep 0.3
check "esc goes back to the list" 'screen list | grep -q "1 shown"'
t send-keys -t smoke:list e
sleep 0.2
t send-keys -t smoke:list BSpace BSpace BSpace
t send-keys -t smoke:list -l Kettle
t send-keys -t smoke:list Enter
sleep 0.3
check "e renames it" 'screen list | grep -q "Kettle" && ! screen list | grep -q "Tea"'
t send-keys -t smoke:list x
sleep 0.3
check "x cancels it" 'screen list | grep -q "No clocks"'
t send-keys -t smoke:list q
sleep 0.4
check "q quits the list with 0 when nothing runs" '[[ $(exit_code list) == 0 ]]'

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
