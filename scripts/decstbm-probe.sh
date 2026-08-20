#!/usr/bin/env bash
# A deterministic stand-in for apt's bottom-anchored progress bar.
#
# apt reserves the last row with DECSTBM, parks the cursor with save/restore,
# and repaints that row OUT OF BAND while ordinary output scrolls above it.
# That trio -- scroll region, cursor save/restore, bottom-row repaint -- is what
# a plain `ls` or a scrolling build log never touches, and it is the trio the
# [11.0] queue entry needs compared against a bare terminal emulator.
#
# Deliberately NOT apt: this needs no root, no network and no package state, so
# it is repeatable on any host and cannot be blamed on a mirror.
#
# Usage: decstbm-probe.sh [lines] [delay-seconds]
set -u
COUNT=${1:-40}
DELAY=${2:-0.05}

rows=$(tput lines 2>/dev/null || echo 24)
cols=$(tput cols 2>/dev/null || echo 80)
last=$rows
top=$((rows - 1))

cleanup() { printf '\e[r\e[%d;1H\e[?25h\n' "$rows"; }
trap cleanup EXIT INT TERM

printf '\e[2J\e[H'                 # clear, home
printf '\e[1;%dr' "$top"           # DECSTBM: reserve the last row
printf '\e[?25l'                   # hide cursor, as apt does

bar_width=$((cols - 12))
[ "$bar_width" -lt 10 ] && bar_width=10

for i in $(seq 1 "$COUNT"); do
    # ordinary scrolling output, inside the region
    printf '\e[%d;1H' "$top"
    printf 'line %3d  the quick brown fox jumps over the lazy dog\n' "$i"

    # the out-of-band bottom-row repaint: save, leave the region, paint, restore
    pct=$(( i * 100 / COUNT ))
    filled=$(( pct * bar_width / 100 ))
    printf '\e7'                                   # DECSC save cursor
    printf '\e[r'                                  # release region to address row $last
    printf '\e[%d;1H' "$last"
    printf '\e[7m %3d%% [' "$pct"
    j=0; while [ $j -lt $filled ]; do printf '#'; j=$((j+1)); done
    while [ $j -lt $bar_width ]; do printf '.'; j=$((j+1)); done
    printf ']\e[0m\e[K'
    printf '\e[1;%dr' "$top"                       # re-arm the region
    printf '\e8'                                   # DECRC restore cursor
    sleep "$DELAY"
done
printf '\e7\e[r\e[%d;1H\e[2K PROBE COMPLETE — %d lines, %dx%d\e8' "$last" "$COUNT" "$cols" "$rows"
sleep 1
