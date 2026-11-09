#!/bin/bash
# drives the prompt in a private tmux server running plain bash
B=$PWD/target/release/tty-prompt-spike
SD=/tmp/claude-1000/-home-paulo-projects-personal-shell-ai/b0303105-a0ff-4336-9ebc-4e9df46c676e/scratchpad
T="tmux -L spike2"
start() { $T kill-session -t t 2>/dev/null; $T new-session -d -s t -x ${1:-70} -y 24 "bash --norc --noprofile"; sleep 0.4; }
send() { for k in "$@"; do if [[ $k == lit:* ]]; then $T send-keys -t t -l "${k#lit:}"; else $T send-keys -t t "$k"; fi; sleep 0.25; done; }
run() { # name, cmd-to-run-in-shell, keys...
  local name=$1 cmd=$2; shift 2
  start; send "lit:S0=\$(stty -g); $cmd; echo EXIT=\$?; [ \"\$S0\" = \"\$(stty -g)\" ] && echo TERMIOS=restored || echo TERMIOS=CHANGED" Enter
  sleep 0.5; send "$@"; sleep 0.8
  echo "=== $name"; $T capture-pane -t t -p | grep -v '^$' | tail -${TAIL:-9}
}
case "$1" in
 basic)
  run allow "$B -- rm -rf build" lit:y
  run enter-denies "$B -- ls" Enter
  run esc-denies "$B -- ls" Escape
  run ctrl-c-denies "$B -- ls" C-c
  run arrow-ignored-then-y "$B -- ls" Up Left lit:y
  run unknown-key-denies "$B -- ls" lit:x ;;
 flush)
  run pretyped-flushed "sleep 2; $B -- ls" lit:y lit:y lit:y   # typed before prompt exists
  sleep 0; ;;
 hostile)
  TAIL=14 run hostile "$B -- \$'echo ok\\e]0;pwned\\a\\e[2J\\e[31m\\u202erm -rf /\\n\\x7f'" n ;;
 high)
  run high-yes "$B --risk high -- rm x" lit:y lit:e lit:s Enter
  run high-y-only "$B --risk high -- rm x" lit:y Enter
  run high-Y-es-typo "$B --risk high -- rm x" lit:y lit:e lit:s lit:s Enter ;;
 paths)
  run panic "$B --panic -- ls" 
  run sigterm "B=$B SIG=TERM bash kill.sh"
  run sigkill "B=$B SIG=KILL bash kill.sh"
  run no-tty "setsid -w $B -- ls </dev/null" ;;
 wrap)
  TAIL=14 run wrap "$B -- \$(printf 'x%.0s' {1..150})" n ;;
esac
$T kill-session -t t 2>/dev/null
