#!/bin/bash
# PROMPT_MODE=plain|starship  SM_VARIANT=A|B|C  run.sh keys...  (lit: prefix = literal)
SD=/tmp/claude-1000/-home-paulo-projects-personal-shell-ai/b0303105-a0ff-4336-9ebc-4e9df46c676e/scratchpad
T="tmux -L spike6"
rm -f $SD/bash-spike.* $SD/spike-env.log
$T kill-session -t b 2>/dev/null
cat > $SD/bashrc <<RC
PS1='\\w \\$ '
[[ "$PROMPT_MODE" == starship ]] && eval "\$(starship init bash)"
source /home/paulo/projects/personal/shell-ai/spikes/bash/glue.bash
[[ -n "$VI" ]] && set -o vi
RC
$T new-session -d -s b -x 70 -y 40 "env SPIKE_DIR=$SD SPIKE_LOG=$SD/spike-env.log PROMPT_MODE=$PROMPT_MODE SM_VARIANT=$SM_VARIANT VI=$VI SPIKE_MODE=$SPIKE_MODE bash --noprofile --rcfile $SD/bashrc -i; sleep 1"
sleep 1
for k in "$@"; do if [[ $k == lit:* ]]; then $T send-keys -t b -l "${k#lit:}"; else $T send-keys -t b $k; fi; sleep ${KW:-0.6}; done
sleep ${WAIT:-0.5}
$T capture-pane -t b -p | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}' | tail -${TAIL:-12}
echo "--- log"; cat $SD/bash-spike.log 2>/dev/null | cut -c1-140; echo "--- env"; tr '\0' '|' < $SD/spike-env.log 2>/dev/null | tr '\n' ' ' | head -c 200; echo
$T kill-session -t b 2>/dev/null
