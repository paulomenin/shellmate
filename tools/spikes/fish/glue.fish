set -g __sm_stub (dirname (status filename))/stub.sh
function __sm_widget --argument-names action
    # prelude: move below the whole commandline and clear (pattern from fish's __fish_echo)
    set -l line (commandline --line)
    string >&2 repeat -N \n --count=(math (commandline | count) - $line + 1)
    echo -n \e\[J >&2
    set -l buf (commandline | string collect)
    set -l cur (commandline -C)
    set -l out (printf 'v=1\0action=%s\0shell=fish\0cursor=%s\0buffer=%s\0' $action $cur "$buf" | $__sm_stub | string collect)
    set -l rc $pipestatus[2]
    echo "rc=$rc" >> /tmp/claude-1000/-home-paulo-projects-personal-shell-ai/b0303105-a0ff-4336-9ebc-4e9df46c676e/scratchpad/spike-rc.log
    switch $rc
        case 0
            commandline -r -- $out
            commandline -C (string length -- $out)
        case 10
        case '*'
    end
    # postlude: leave room for the prompt above the (redrawn) commandline, then repaint
    string >&2 repeat -N \n --count=(math (count (fish_prompt)) - 1)
    string >&2 repeat -N \n --count=(math $line - 1)
    commandline -f repaint
end
bind \eg '__sm_widget generate'
bind \ex '__sm_widget explain'
bind \em '__sm_widget multiline'
function __sm_pre --on-event fish_preexec
    echo "pre:$argv" >> /tmp/claude-1000/-home-paulo-projects-personal-shell-ai/b0303105-a0ff-4336-9ebc-4e9df46c676e/scratchpad/spike-hooks.log
end
function __sm_post --on-event fish_postexec
    echo "post:status=$status pipestatus=$pipestatus dur=$CMD_DURATION cmd=$argv" >> /tmp/claude-1000/-home-paulo-projects-personal-shell-ai/b0303105-a0ff-4336-9ebc-4e9df46c676e/scratchpad/spike-hooks.log
end
