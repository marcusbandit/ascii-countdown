#!/usr/bin/env bash
# Makes the README images from a real run: tock in a headless tmux, captured
# with colours and rendered by ansi2png.py.
#
#   scripts/screenshot.sh [path/to/tock]
#
# Writes docs/countdown.png, docs/stopwatch.png, docs/list.png, docs/multi.png
# and docs/flash.gif.
set -euo pipefail
cd "$(dirname "$0")/.."

bin=$(realpath "${1:-target/release/tock}")
sock=tock-shot-$$
work=$(mktemp -d)
t() { tmux -L "$sock" -f /dev/null "$@"; }
trap 't kill-server 2>/dev/null || true; rm -rf "$work"' EXIT
mkdir -p docs

cols=100 rows=26
t new-session -d -s shot -x $cols -y $rows -n idle
t set -g status off
t new-window -d -t shot: -n cd "$bin --title 'Focus' 25m"
t new-window -d -t shot: -n sw "$bin --title 'Lap 3' stopwatch"
t new-window -d -t shot: -n end "$bin --no-bell --title 'Tea' 4s"
# The clock list gets a bigger window of its own, filled in through the form.
t new-session -d -s list -x 120 -y 30 "$bin"
lk() { t send-keys -t list "$@"; sleep 0.15; }
add() {
  lk a
  lk -l "$1"
  if [[ $2 == stopwatch ]]; then lk Tab l; else lk Tab Tab; lk -l "$2"; fi
  lk Enter
}

frame=0
grab_end() {
  t capture-pane -e -N -p -t shot:end > "$work/end-$(printf %03d $frame).txt"
  frame=$((frame + 1))
}

# 2.4 s in: the countdown reads 24:58 and the stopwatch 00:02.x.
for _ in $(seq 12); do grab_end; sleep 0.2; done
t capture-pane -e -N -p -t shot:cd > "$work/cd.txt"
t capture-pane -e -N -p -t shot:sw > "$work/sw.txt"
# Keep grabbing the short timer through zero and the flashing.
for _ in $(seq 34); do grab_end; sleep 0.2; done

# Now fill the list through the form, and show four of the clocks.
sleep 0.4
add Tea 4m
add Pasta 9m30s
add Run stopwatch
add Bake 1h
add Laundry 45m
lk k
t capture-pane -e -N -p -t list: > "$work/list.txt"
# On to the display, then one step left so the selection frame shows.
lk Enter Space h
sleep 0.3
t capture-pane -e -N -p -t list: > "$work/multi.txt"

python3 scripts/ansi2png.py "$work/cd.txt" docs/countdown.png --cols $cols --rows $rows
python3 scripts/ansi2png.py "$work/sw.txt" docs/stopwatch.png --cols $cols --rows $rows
python3 scripts/ansi2png.py "$work/list.txt" docs/list.png --cols 120 --rows 30
python3 scripts/ansi2png.py "$work/multi.txt" docs/multi.png --cols 120 --rows 30
# Frames grabbed before the first draw are empty; leave them out.
mapfile -t frames < <(grep -l '█' "$work"/end-*.txt)
python3 scripts/ansi2png.py --gif docs/flash.gif "${frames[@]}" --cols $cols --rows $rows --delay 200
ls -la docs
