zmodload zsh/datetime
source ./glue.zsh 2>/dev/null
a=$(<_/proc/sys/kernel/ns_last_pid) 2>/dev/null
