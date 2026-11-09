#!/bin/bash
$B --hold -- ls &
P=$!
sleep 0.6
kill -$SIG $P
wait $P
