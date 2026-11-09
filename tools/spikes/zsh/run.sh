#!/bin/bash
# usage: PROMPT=starship|plain run.sh 'keys...'   (prefix lit: = literal text)
SD=/tmp/claude-1000/-home-paulo-projects-personal-shell-ai/b0303105-a0ff-4336-9ebc-4e9df46c676e/scratchpad
T="tmux -L spike5"
rm -f $SD/zsh-spike.log $SD/spike-*.log
$T kill-session -t z 2>/dev/null
cat > $SD/zshrc <<RC
PS1='%~ %# '
[[ "\$PMODE" == starship ]] && eval "\$(starship init zsh)"
source /home/paulo/projects/personal/shell-ai/spikes/zsh/glue.zsh
[[ -n "\$VI" ]] && bindkey -v
RC
mkdir -p $SD/zdot; cp $SD/zshrc $SD/zdot/.zshrc
$T new-session -d -s z -x 70 -y 40 "env ZDOTDIR=$SD/zdot SPIKE_DIR=$SD SPIKE_LOG=$SD/spike-env.log PMODE=$PROMPT_MODE SM_VARIANT=$SM_VARIANT VI=$VI SPIKE_MODE=$SPIKE_MODE zsh -d -i; sleep 1"
sleep 1
for k in "$@"; do if [[ $k == lit:* ]]; then $T send-keys -t z -l "${k#lit:}"; else $T send-keys -t z $k; fi; sleep ${KW:-0.6}; done
sleep ${WAIT:-0.5}
$T capture-pane -t z -p | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}' | tail -${TAIL:-12}
echo "--- log"; cat $SD/zsh-spike.log 2>/dev/null | cut -c1-140; echo "--- env"; tr '\0' '|' < $SD/spike-env.log 2>/dev/null | tr '\n' ' ' | head -c 200; echo
$T kill-session -t z 2>/dev/null
