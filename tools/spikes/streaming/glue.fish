set -g __sm_bin (dirname (status filename))/target/release/streaming-spike
function __sm_stream
    set -l line (commandline --line)
    string >&2 repeat -N \n --count=(math (commandline | count) - $line + 1)
    echo -n \e\[J >&2
    $__sm_bin "http://127.0.0.1:18080/?delay=0.04&n=50" 2>>$SPIKE_ERR
    set -l rc $status
    string >&2 repeat -N \n --count=(math (count (fish_prompt)) - 1)
    string >&2 repeat -N \n --count=(math $line - 1)
    commandline -f repaint
    echo "rc=$rc" >> $SPIKE_ERR
end
bind alt-a __sm_stream
