#!/bin/bash
# Runs the conformance suite for every installed shell, with and without starship.
cd "$(dirname "$0")/../.."
fail=0
for sh in fish zsh bash; do
  command -v "$sh" >/dev/null || { echo "skipping $sh (not installed)"; continue; }
  for mode in plain starship; do
    [ "$mode" = starship ] && ! command -v starship >/dev/null && continue
    tools/e2e/conformance.sh "$sh" "$mode" | grep -E "FAIL|passed|\|" || true
    [ "${PIPESTATUS[0]}" -eq 0 ] || fail=1
  done
done
exit $fail
