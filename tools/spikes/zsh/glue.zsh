zmodload zsh/datetime
typeset -g __sm_dir=${0:A:h}
typeset -g __sm_stub=$__sm_dir/stub.sh
typeset -g __sm_stream=$__sm_dir/../streaming/target/release/streaming-spike
typeset -g __sm_log=${SPIKE_DIR:-/tmp}/zsh-spike.log

__sm_widget() {
  local action=$1 out rc
  zle -I                                   # move below the commandline, invalidate display
  out=$(printf 'v=1\0action=%s\0shell=zsh\0cursor=%s\0buffer=%s\0' $action $CURSOR "$BUFFER" | $__sm_stub; printf 'x')
  # exit code of the stub is lost in $(...; printf x); capture it separately:
  rc=$?
  print -r -- "rc(naive)=$rc" >> $__sm_log
  out=${out%x}
  zle reset-prompt
}
__sm_widget2() {   # proper exit code capture: run stub inside, echo rc as sentinel
  local action=$1 out rc
  zle -I
  out=$(printf 'v=1\0action=%s\0shell=zsh\0cursor=%s\0buffer=%s\0' $action $CURSOR "$BUFFER" | $__sm_stub; print -n "x$?")
  rc=${out##*x}; out=${out%x*}
  print -r -- "rc=$rc" >> $__sm_log
  case $rc in
    0)  BUFFER=$out; CURSOR=${#BUFFER} ;;
    10) ;;
  esac
  case ${SM_VARIANT:-A} in
    A) zle reset-prompt ;;
    B) zle redisplay ;;
    C) print -n '\n'; zle reset-prompt ;;          # one extra newline
    D) zle -R ;;
    E) zle reset-prompt; zle -R ;;
    F) zle -I; zle reset-prompt ;;                  # second -I after the output
  esac
}
__sm_gen()  { __sm_widget2 generate }
__sm_exp()  { __sm_widget2 explain }
__sm_ml()   { __sm_widget2 multiline }
__sm_stream_w() {
  zle -I
  SPIKE_MODE=${SPIKE_MODE:-sigint} $__sm_stream "http://127.0.0.1:18080/?delay=0.04&n=50" 2>>$__sm_log
  print -r -- "stream rc=$?" >> $__sm_log
  zle reset-prompt
}
zle -N __sm_gen; zle -N __sm_exp; zle -N __sm_ml; zle -N __sm_stream_w
for km in emacs viins vicmd; do
  bindkey -M $km '^[g' __sm_gen
  bindkey -M $km '^[x' __sm_exp
  bindkey -M $km '^[m' __sm_ml
  bindkey -M $km '^[a' __sm_stream_w
done
# hooks
typeset -ga __sm_ring
__sm_preexec() { __sm_cmd=$1; __sm_t0=$EPOCHREALTIME }
__sm_precmd()  { local st=$?; print -r -- "post: status=$st dur=$(( EPOCHREALTIME - __sm_t0 )) cmd=$__sm_cmd" >> $__sm_log; __sm_cmd= }
autoload -Uz add-zsh-hook
add-zsh-hook preexec __sm_preexec
add-zsh-hook precmd __sm_precmd
