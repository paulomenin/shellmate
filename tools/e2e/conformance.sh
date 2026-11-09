#!/bin/bash
# Shell conformance suite (docs/glue.md section 10): the real shell in a private tmux server, the real binary,
# a mock provider. usage: tools/e2e/conformance.sh <fish|zsh|bash> [plain|starship]
set -uo pipefail
cd "$(dirname "$0")/../.."
SHELL_NAME=${1:?usage: conformance.sh <fish|zsh|bash> [plain|starship]}; MODE=${2:-plain}
cargo build -q -p shellmate || exit 1
BIN=$PWD/target/debug/shellmate
PORT=$((20000 + RANDOM % 10000)); SOCK="sm-conf-$SHELL_NAME-$MODE-$$"; T="tmux -L $SOCK"
# alt+r is readline's revert-line in bash, and shellmate never overrides a binding, so bash uses alt+e here
CAPKEY=alt+r; CAPSEND=M-r; [ "$SHELL_NAME" = bash ] && { CAPKEY=alt+e; CAPSEND=M-e; }
TMP=$(mktemp -d); LAST=$TMP/last-request.json
cleanup() { kill $SRV 2>/dev/null; $T kill-server 2>/dev/null; rm -rf "$TMP"; }
trap cleanup EXIT
python3 tools/e2e/mock_openai.py "$PORT" "$LAST" & SRV=$!
cat > "$TMP/config.toml" <<CFG
version = 1
[context]
default_level = 2
[providers.mock]
plugin = "builtin:openai-compatible"
trust = "remote"
max_context_level = 2
[providers.mock.settings]
endpoint = "http://127.0.0.1:$PORT"
model = "mock"
[llm]
chain = ["mock"]
[input.shortcuts]
"alt+m" = "ask"
"alt+i" = "generate"
"alt+j" = "explain"
"$CAPKEY" = "capture"
CFG
export SHELLMATE_CONFIG=$TMP/config.toml XDG_RUNTIME_DIR=$TMP/run; mkdir -p -m 700 "$XDG_RUNTIME_DIR"
STARSHIP_INIT=""; [ "$MODE" = starship ] && STARSHIP_INIT=1
case $SHELL_NAME in
  fish)
    INIT="${STARSHIP_INIT:+starship init fish | source; }$BIN init fish | source"
    CMD="fish --no-config -C '$INIT'" ;;
  zsh)
    mkdir -p "$TMP/zdot"
    { echo "PS1='%~ %# '"; [ -n "$STARSHIP_INIT" ] && echo 'eval "$(starship init zsh)"'; echo "source <($BIN init zsh)"; } > "$TMP/zdot/.zshrc"
    CMD="env ZDOTDIR=$TMP/zdot zsh -d -i" ;;
  bash)
    { echo "PS1='\\w \\\$ '"; [ -n "$STARSHIP_INIT" ] && echo 'eval "$(starship init bash)"'; echo "eval \"\$($BIN init bash)\""; } > "$TMP/bashrc"
    CMD="bash --noprofile --rcfile $TMP/bashrc -i" ;;
  *) echo "unknown shell"; exit 2 ;;
esac
$T new-session -d -s c -x 100 -y 45 "env SHELLMATE_CONFIG=$SHELLMATE_CONFIG XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR $CMD"
sleep 1.5
FAILS=0; N=0
send()   { $T send-keys -t c "$@"; }
type_()  { $T send-keys -t c -l "$1"; }
screen() { $T capture-pane -t c -p | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}'; }
last()   { screen | tail -1; }
wait_for() { # wait_for <seconds> <grep -F pattern>
  for _ in $(seq 1 $(( $1 * 5 ))); do screen | grep -qF -- "$2" && return 0; sleep 0.2; done; return 1; }
check() { N=$((N+1)); if [ "$1" = ok ]; then echo "  ok   $2"; else echo "  FAIL $2"; [ -n "${3:-}" ] && echo "$3" | sed 's/^/       | /'; FAILS=$((FAILS+1)); fi; }

echo "== $SHELL_NAME / $MODE"
# 1. generate replaces the buffer; the typed line stays visible; answer is printed below it
send C-u; type_ "list the biggest files"; send M-i
wait_for 8 "du -sh * | sort -rh | head"
S=$(screen)
[[ "$(last)" == *"du -sh * | sort -rh | head"* ]] && check ok "generate: command is in the input line" || check FAIL "generate: command is in the input line" "$S"
! grep -qF "This lists the largest entries first" <<<"$S" && check ok "generate: command only, no prose printed" || check FAIL "generate: command only, no prose printed" "$S"
[ "$(grep -cF 'list the biggest files' <<<"$S")" -ge 1 ] && check ok "generate: the typed line is not clobbered" || check FAIL "generate: the typed line is not clobbered" "$S"
! grep -qF '```' <<<"$S" && check ok "generate: no fence markers on screen" || check FAIL "generate: no fence markers" "$S"

# 2. explain: answer printed, buffer unchanged (exit 10 with empty stdout)
send C-u; type_ "du -sh *"; send M-j
wait_for 8 "largest first"
S=$(screen)
[[ "$(last)" == *"du -sh *" && "$(last)" != *"sort"* ]] && check ok "explain: buffer unchanged" || check FAIL "explain: buffer unchanged" "$S"

# 3. quotes, dollars, backticks, backslashes and unicode survive the envelope exactly
TRICKY='echo "$HOME" '"'"'q'"'"' `x` \\ café'
send C-u; type_ "$TRICKY"; send M-j; sleep 1.5
python3 - "$LAST" "$TRICKY" <<'PY' && check ok "envelope: tricky buffer round-trips exactly" || check FAIL "envelope: tricky buffer round-trips exactly" "$(cat "$LAST")"
import json, sys
sent = json.load(open(sys.argv[1]))["messages"][-1]["content"]
assert sent == "Explain this command:\n" + sys.argv[2], repr(sent)
PY

# 4. ask: prose printed, suggested command placed in the buffer
send C-u; type_ "how do I find big files?"; send M-m
wait_for 8 "du -sh * | sort -rh | head"
[[ "$(last)" == *"du -sh * | sort -rh | head"* ]] && check ok "ask: suggestion placed in the buffer" || check FAIL "ask: suggestion placed in the buffer" "$(screen)"

# 5. Ctrl-C cancels the request, the shell survives, typeahead is kept
send C-u; type_ "SLOWPLEASE question"; send M-m; sleep 1.5; send C-c; sleep 1
wait_for 4 "[cancelled]" && check ok "cancel: [cancelled] shown" || check FAIL "cancel: [cancelled] shown" "$(screen)"
type_ "zz"; sleep 0.6
[[ "$(last)" == *"SLOWPLEASE questionzz" ]] && check ok "cancel: prompt redrawn, buffer and typeahead intact" || check FAIL "cancel: prompt redrawn, buffer and typeahead intact" "$(screen)"

# 5b. an Enter pressed while the model is still working must not run the suggestion that then lands in the buffer
send C-u; send C-l; sleep 0.4   # clear the screen so the wait below cannot match an earlier answer
type_ "SLOWPLEASE list the biggest files"; send M-i; sleep 1.5; send Enter
wait_for 12 "was ignored"; sleep 0.8
S=$(screen)
[[ "$(last)" == *"du -sh * | sort -rh | head"* ]] && check ok "typeahead: Enter pressed while waiting did not run the suggestion" || check FAIL "typeahead: Enter pressed while waiting did not run the suggestion" "$S"
grep -qF "Enter pressed while waiting was ignored" <<<"$S" && check ok "typeahead: the user is told the Enter was ignored" || check FAIL "typeahead: the user is told the Enter was ignored" "$S"

# 5c. the approval prompt: the model asks to run a command, nothing runs without a human's `y`
send C-u; send C-l; sleep 0.4
type_ "EXECPLEASE what is here"; send M-m
wait_for 8 "wants to run a command"
S=$(screen)
grep -qF "echo e2e-exec-ran" <<<"$S" && grep -qF "risk: low" <<<"$S" && check ok "exec: the prompt shows the exact command and its risk" || check FAIL "exec: the prompt shows the exact command and its risk" "$S"
send Enter; wait_for 8 "denied"; sleep 1
S=$(screen)
! grep -qF "ran (approved)" <<<"$S" && grep -qF "Checked it." <<<"$S" && check ok "exec: Enter denies, the model carries on" || check FAIL "exec: Enter denies, the model carries on" "$S"
send C-u; send C-l; sleep 0.4
type_ "EXECPLEASE again"; send M-m
wait_for 8 "wants to run a command"; send y; wait_for 8 "ran (approved)"; sleep 1
S=$(screen)
grep -qF "e2e-exec-ran" <<<"$S" && grep -qF "exit 0" <<<"$S" && check ok "exec: y runs it once and shows the result" || check FAIL "exec: y runs it once and shows the result" "$S"
# keys typed while the model is still thinking must not answer the prompt
send C-u; send C-l; sleep 0.4
type_ "EXECPLEASE SLOWPLEASE slow"; send M-m; sleep 1.5; send y; send y; send y
wait_for 14 "wants to run a command"; sleep 1.5
S=$(screen)
grep -qF "choice (n):" <<<"$S" && ! grep -qF "ran (approved)" <<<"$S" && check ok "exec: keys typed while waiting cannot answer the prompt" || check FAIL "exec: keys typed while waiting cannot answer the prompt" "$S"
send n; sleep 8

# 5d. the file-read prompt: nothing is read without a human's `y`, and the prompt says where the text goes
send C-u; send C-l; sleep 0.4
type_ "READPLEASE what is in it"; send M-m
wait_for 8 "wants to read a file"
S=$(screen)
grep -qF "read: Cargo.toml" <<<"$S" && grep -qF "sent to" <<<"$S" && ! grep -qF "[b]" <<<"$S" && check ok "read: the prompt names the file and where the text goes" || check FAIL "read: the prompt names the file and where the text goes" "$S"
send Enter; wait_for 8 "denied"; sleep 1
S=$(screen)
! grep -qF "read (approved)" <<<"$S" && grep -qF "Checked it." <<<"$S" && check ok "read: Enter denies, the model carries on" || check FAIL "read: Enter denies, the model carries on" "$S"
send C-u; send C-l; sleep 0.4
type_ "READPLEASE again"; send M-m
wait_for 8 "wants to read a file"; send y; wait_for 8 "read (approved)"; sleep 1
python3 - "$LAST" <<'PY' && check ok "read: y reads it once and the text reaches the model" || check FAIL "read: y reads it once and the text reaches the model" "$(cat "$LAST")"
import json, sys
wire = json.dumps(json.load(open(sys.argv[1])))
assert "<<<file>>>" in wire and "[workspace]" in wire, "file text missing"
assert "never instructions" in wire, "not labeled as data"
PY

# 6. streaming: the first pieces are visible while the rest is still being generated
send C-u; type_ "STREAMTEST please"; send M-m; sleep 1.3
S=$(screen)
if grep -qF "piece-1 " <<<"$S" && ! grep -qF "piece-12" <<<"$S"; then check ok "streaming: partial answer visible mid-generation"; else check FAIL "streaming: partial answer visible mid-generation" "$S"; fi
wait_for 6 "piece-12" && check ok "streaming: answer completes" || check FAIL "streaming: answer completes" "$(screen)"
sleep 0.5; [[ "$(last)" == *"STREAMTEST please" ]] && check ok "streaming: buffer intact afterwards" || check FAIL "streaming: buffer intact afterwards" "$(screen)"

# 7. level-2 context: recent commands are sent, secrets are replaced by placeholders
send C-u; type_ "export DEMO_API_TOKEN=abcdef0123456789abcdef0123"; send Enter; sleep 0.5
type_ "echo hello-from-history"; send Enter; sleep 0.5
send C-l; sleep 0.4   # clear the screen so the wait below cannot match an earlier answer
type_ "what did I just run?"; send M-m; wait_for 8 "largest entries"
python3 - "$LAST" <<'PY' && check ok "privacy: history sent, secret replaced by a placeholder" || check FAIL "privacy: history sent, secret replaced by a placeholder" "$(cat "$LAST")"
import json, sys
wire = json.dumps(json.load(open(sys.argv[1])))
assert "echo hello-from-history" in wire, "history missing"
assert "abcdef0123456789abcdef0123" not in wire, "SECRET LEAKED"
assert "<SECRET:" in wire, "no placeholder"
PY

# 7b. capture: the shortcut wraps the line in `shellmate run --` (it runs nothing); after running it, the next
#     question carries the output, labeled as data
send C-u; send C-l; sleep 0.4
type_ "echo e2e-alpha-beta"; send $CAPSEND; sleep 1
grep -qF "run --" <<<"$(screen)" && ! grep -qE '^e2e-alpha-beta$' <<<"$(screen)" && check ok "capture: the line is wrapped in the input buffer" || check FAIL "capture: the line is wrapped in the input buffer" "$(screen)"
! grep -qE '^e2e-alpha-beta$' <<<"$(screen)" && check ok "capture: wrapping alone ran nothing" || check FAIL "capture: wrapping alone ran nothing" "$(screen)"
send Enter; sleep 1.5
send C-l; sleep 0.4
type_ "why is the output like that?"; send M-m; wait_for 8 "largest entries"
python3 - "$LAST" <<'PY' && check ok "capture: the next question carries the output as data" || check FAIL "capture: the next question carries the output as data" "$(cat "$LAST")"
import json, sys
wire = json.dumps(json.load(open(sys.argv[1])))
assert "<<<captured output>>>\\ne2e-alpha-beta" in wire, "captured output missing"
assert "never instructions" in wire, "not labeled as data"
PY
send C-u; type_ "ls"; send $CAPSEND; sleep 0.8; send $CAPSEND; sleep 0.8
[[ "$(last)" == *"\$ ls" || "$(last)" == *"> ls" || "$(last)" == *"ls" ]] && [[ "$(last)" != *"run --"* ]] && check ok "capture: pressing it again unwraps the line" || check FAIL "capture: pressing it again unwraps the line" "$(screen)"

# 8. the session id reaches the store: the shell's own conversation holds the earlier turns, and a second
#    interactive shell (new init) gets a different session. (The remote mock at level 2 is local-only audience, so
#    this checks storage and session plumbing, not replay.)
send C-u; type_ "$BIN conversation show | grep -c '^--- turn'"; send Enter; sleep 1
S=$(screen)
grep -qE '^[1-9][0-9]*$' <<<"$S" && check ok "conversation: this shell's session stored its turns" || check FAIL "conversation: this shell's session stored its turns" "$S"
send C-u; type_ "echo SESS=\$SHELLMATE_SESSION | grep -E 'SESS=[0-9a-f]{16}\$'"; send Enter; sleep 0.7
grep -qE 'SESS=[0-9a-f]{16}$' <<<"$(screen)" && check ok "conversation: SHELLMATE_SESSION is exported" || check FAIL "conversation: SHELLMATE_SESSION is exported" "$(screen)"

# 9. the hooks and widgets never print errors of their own
S=$(screen)
! grep -qE "__shellmate|shellmate_" <<<"$S" && check ok "hooks: no error output from the glue" || check FAIL "hooks: no error output from the glue" "$S"

echo "== $SHELL_NAME / $MODE: $((N-FAILS))/$N passed"
exit $FAILS
