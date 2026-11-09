#!/bin/bash
# t.sh <shell-cmd> [keys-after-seconds key...]; runs in a private tmux server, prints the final screen
B=$PWD/target/release/streaming-spike
T="tmux -L spike3"
case="$1"; wait="${2:-4}"; shift 2
$T kill-session -t s 2>/dev/null
$T new-session -d -s s -x ${COLS:-70} -y 40 "${SHELLCMD:-bash --norc --noprofile}"; sleep 0.5
$T send-keys -t s -l "$case"; $T send-keys -t s Enter
for k in "$@"; do sleep ${KEYWAIT:-1.2}; $T send-keys -t s "$k"; done
sleep $wait; $T capture-pane -t s -p | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}'
$T kill-session -t s
