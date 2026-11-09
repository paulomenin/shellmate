__sm_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
__sm_stub=$__sm_dir/stub.sh
__sm_stream=$__sm_dir/../streaming/target/release/streaming-spike
__sm_log=${SPIKE_DIR:-/tmp}/bash-spike.log
__sm_tmp=${SPIKE_DIR:-/tmp}/bash-spike.out

# --- widget: envelope via stdin, stdout to a file (no $(...) subshell), status read directly
__sm_widget() {
  local action=$1 rc out
  case ${SM_VARIANT:-A} in
    A|B|C) printf '\n' >/dev/tty ;;
    F) local p=${PS1@P}; p=${p//[$'\001'$'\002']/}; printf '\r\e[K%s%s' "${p##*$'\n'}" "$READLINE_LINE" >/dev/tty ;;   # restore the line readline cleared
  esac
  printf 'v=1\0action=%s\0shell=bash\0cursor=%s\0buffer=%s\0' "$action" "$READLINE_POINT" "$READLINE_LINE" | "$__sm_stub" >"$__sm_tmp"
  rc=${PIPESTATUS[1]}
  printf 'rc=%s\n' "$rc" >>"$__sm_log"
  if [[ $rc == 0 ]]; then
    IFS= read -r -d '' out <"$__sm_tmp"      # builtin, no fork; keeps trailing newlines as it reads to EOF
    READLINE_LINE=$out; READLINE_POINT=${#out}
  fi
  case ${SM_VARIANT:-A} in
    A|F|H) ;;                                              # rely on readline's own redisplay
    B) printf '\r\e[K%s' "${PS1@P}" >/dev/tty ;;       # print the expanded prompt ourselves
    C) printf '\e[1A' >/dev/tty ;;                     # move up one line so readline redraws over its old line
  esac
}
__sm_gen() { __sm_widget generate; }
__sm_exp() { __sm_widget explain; }
__sm_ml()  { __sm_widget multiline; }
__sm_stream_w() {
  printf '\n' >/dev/tty
  SPIKE_MODE=${SPIKE_MODE:-sigint} "$__sm_stream" "http://127.0.0.1:18080/?delay=0.04&n=50" 2>>"$__sm_log"
  printf 'stream rc=%s\n' "$?" >>"$__sm_log"
}
for km in emacs vi-insert vi-command; do
  bind -m $km -x '"\em": __sm_gen'
  bind -m $km -x '"\ej": __sm_exp'
  bind -m $km -x '"\ek": __sm_ml'
  bind -m $km -x '"\ei": __sm_stream_w'
done

# --- hooks without forks: PS0 (bash >= 4.4) runs before each command; PROMPT_COMMAND after
PS0='${__sm_x[$((__sm_t0=${EPOCHREALTIME/./}))]:0:0}'
__sm_pc() {
  local st=$?
  local hl                                   # last history line, via a file redirect (no subshell)
  history 1 >"$__sm_tmp.h"
  IFS= read -r hl <"$__sm_tmp.h"
  hl=${hl#"${hl%%[![:space:]]*}"}                     # trim leading spaces
  local hn=${hl%%[![:digit:]]*}                        # history number
  hl=${hl#"$hn"}; hl=${hl#"${hl%%[![:space:]]*}"}
  if [[ $hn == "$__sm_last_hn" ]]; then hl='<not recorded in history>'; fi   # HISTCONTROL=ignorespace / ignoredups
  __sm_last_hn=$hn
  if [[ -n $__sm_t0 ]]; then
    printf 'post: status=%s dur_us=%s cmd=%s\n' "$st" "$(( ${EPOCHREALTIME/./} - __sm_t0 ))" "$hl" >>"$__sm_log"
    __sm_t0=
  fi
}
if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == "declare -a"* ]]; then PROMPT_COMMAND+=(__sm_pc); else PROMPT_COMMAND="__sm_pc${PROMPT_COMMAND:+;$PROMPT_COMMAND}"; fi
__sm_sleep() { sleep 2; }
bind -x '"\er": __sm_sleep'
