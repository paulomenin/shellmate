#!/bin/sh
# Fake shellmate: reads NUL-separated envelope on stdin, writes UI to /dev/tty,
# new buffer to stdout, exit code 0 replace / 10 unchanged.
env=$(tr '\0' '\n')
echo "$env" > "${SPIKE_LOG:-/tmp/claude-1000/-home-paulo-projects-personal-shell-ai/b0303105-a0ff-4336-9ebc-4e9df46c676e/scratchpad/spike-env.log}"
action=$(printf '%s\n' "$env" | sed -n 's/^action=//p')
printf '\n[shellmate] thinking (%s)...\n' "$action" > /dev/tty
case "$action" in
  generate) printf '%s' 'ls -la "my dir" | grep '"'"'x'"'"; exit 0 ;;
  explain)  printf '[shellmate] this lists files\n' > /dev/tty; exit 10 ;;
  multiline) printf 'echo one\necho two'; exit 0 ;;
esac
exit 10
