#!/bin/sh
# Mutation testing: change the code on purpose and check that a test fails.
#
#   tools/mutants.sh                 the security modules (about 30 minutes)
#   tools/mutants.sh ARGS...         anything else, passed to cargo-mutants:
#       tools/mutants.sh -p shellmate-core -f 'crates/shellmate-core/src/command/*.rs'
#       tools/mutants.sh --list      only list the mutants
#
# Results are in target/mutants*/mutants.out/ (missed.txt is the list to work through).
set -eu
cd "$(dirname "$0")/.."

if ! cargo mutants --version >/dev/null 2>&1; then
    echo "cargo-mutants is not installed. Run:" >&2
    echo "  mise install" >&2
    exit 1
fi

# instrumented-speed builds are slow: stretch the helper-program time limits
export SHELLMATE_TIME_SCALE="${SHELLMATE_TIME_SCALE:-5}"

if [ $# -gt 0 ]; then
    exec cargo mutants -j 4 --timeout 120 "$@"
fi

cargo mutants -j 4 --timeout 120 -o target/mutants-core -p shellmate-core \
    -f 'crates/shellmate-core/src/command/*.rs' \
    -f 'crates/shellmate-core/src/tools/*.rs' \
    -f 'crates/shellmate-core/src/engine/tool_calls/*.rs' \
    -f 'crates/shellmate-core/src/present/*.rs' \
    -f 'crates/shellmate-core/src/present/rich/*.rs' \
    -f 'crates/shellmate-core/src/render.rs' \
    -f 'crates/shellmate-core/src/sanitize.rs' \
    -f 'crates/shellmate-core/src/secret_cmd.rs' \
    -f 'crates/shellmate-core/src/secrets.rs' \
    -f 'crates/shellmate-core/src/validate/*.rs' || true
cargo mutants -j 4 --timeout 120 -o target/mutants-builtin -p shellmate-builtin \
    -f 'crates/shellmate-builtin/src/middleware_redact/*.rs' \
    -f 'crates/shellmate-builtin/src/tool_man/*.rs' \
    -f 'crates/shellmate-builtin/src/tool_help/*.rs' \
    -f 'crates/shellmate-builtin/src/tool_exec/*.rs' \
    -f 'crates/shellmate-builtin/src/tool_read/*.rs' || true

echo
echo "== summary =="
for d in core redact; do
    f="target/mutants-$d/mutants.out"
    printf '%s: caught %s, missed %s, timeouts %s\n' "$d" \
        "$(wc -l < "$f/caught.txt")" "$(wc -l < "$f/missed.txt")" "$(wc -l < "$f/timeout.txt")"
done
echo "missed mutants are listed in target/mutants-*/mutants.out/missed.txt"
