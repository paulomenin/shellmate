#!/bin/bash
# usage: run.sh 'keys...' ; drives fish in a private tmux server (-L spike)
SD=/tmp/claude-1000/-home-paulo-projects-personal-shell-ai/b0303105-a0ff-4336-9ebc-4e9df46c676e/scratchpad
T="tmux -L spike"
rm -f $SD/spike-*.log
$T kill-session -t sp 2>/dev/null
$T new-session -d -s sp -x 100 -y 20 "fish --no-config -C 'starship init fish | source' -C 'source /home/paulo/projects/personal/shell-ai/spikes/fish/glue.fish'"
sleep 1
for k in "$@"; do if [[ $k == lit:* ]]; then tmux -L spike send-keys -t sp -l "${k#lit:}"; else tmux -L spike send-keys -t sp $k; fi; sleep 0.5; done
tmux -L spike capture-pane -t sp -p | grep -v '^$'
echo "--- rc:"; cat $SD/spike-rc.log 2>/dev/null; echo "--- hooks:"; cat $SD/spike-hooks.log 2>/dev/null
echo "--- env:"; tr '\0' '|' < $SD/spike-env.log 2>/dev/null | head -c 400; echo
