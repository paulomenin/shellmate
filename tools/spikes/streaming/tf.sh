#!/bin/bash
# tf.sh <mode> <keys...>: fish+starship widget; mode=raw|sigint
export SPIKE_ERR=/tmp/claude-1000/-home-paulo-projects-personal-shell-ai/b0303105-a0ff-4336-9ebc-4e9df46c676e/scratchpad/fish-err.log; rm -f $SPIKE_ERR
T="tmux -L spike4"
$T kill-session -t f 2>/dev/null
$T new-session -d -s f -x 70 -y 40 "env SPIKE_MODE=$1 SPIKE_ERR=$SPIKE_ERR fish --no-config -C 'starship init fish | source' -C 'source /home/paulo/projects/personal/shell-ai/spikes/streaming/glue.fish'"; sleep 1
shift
$T send-keys -t f -l "git status"; sleep 0.2; $T send-keys -t f M-a
for k in "$@"; do sleep 1.2; if [[ $k == lit:* ]]; then $T send-keys -t f -l "${k#lit:}"; else $T send-keys -t f $k; fi; done
sleep 4; $T capture-pane -t f -p | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}' | tail -${TAIL:-14}; echo "--- log"; cat $SPIKE_ERR | cut -c1-150
$T kill-session -t f
