---
name: add-shell-or-key-binding
description: Change the shell glue (fish, zsh, bash), add a key binding or action key, or support another shell. Covers the glue templates, key translation, the typeahead flush and the tmux conformance suite.
---

# Shell glue and key bindings

The glue is deliberately thin: collect prompt state, call `shellmate invoke`, put the result in the input line. Logic belongs in Rust. If you are writing more than a few lines of shell, move it into the binary.

## Where things are

- Templates: `crates/shells/{fish/glue.fish,zsh/glue.zsh,bash/glue.bash}`.
- Generation and key translation: `crates/shellmate/src/init.rs` (`parse_key`, one generator per shell, `generate`). Placeholders in the templates are filled there; a test asserts none are left.
- Rc-file install and removal: `install-shell` (`crates/shellmate/src/admin/install_shell.rs`).
- Defaults and the shortcut table: `[input.shortcuts]` in `crates/shellmate-core/src/config/sections.rs`. The built-in default is only `alt+m` ask. The starter config that `setup` and `config init` write (`crates/shellmate/src/admin/templates.rs`) binds all five: `alt+m` ask, `alt+i` generate, `alt+j` explain, `alt+k` new-conversation and `capture` on `alt+r`, or `alt+e` in bash because `alt+r` is readline's revert-line (and `alt+e` is taken in fish by `edit_command_buffer`). shellmate never overrides an existing binding.
- Conformance suite: `tools/e2e/conformance.sh <fish|zsh|bash> [plain|starship]`, `run-all.sh`, and the mock model `mock_openai.py`.

## Rules for glue

- **Never run what the model produced.** The result goes into the buffer only.
- **Discard typeahead** while shellmate works, in every outcome (answer, explain, error, cancel), so a key pressed before the result appeared cannot reach the shell. The conformance suite covers this; keep it covering it.
- **Never override** a binding the user already has. Report a taken key rather than silently replacing it.
- Work with prompt frameworks. The suite runs with and without starship. powerlevel10k, older shells and macOS are untested; do not claim support.
- Keep the shell version floor stated in the docs (`guides/shells.mdx`; bash 4.4 or newer today). Avoid syntax newer than it.
- A failure in the binary must leave the prompt intact and give a one-line message, never a stack trace or a hung prompt.
- No new files outside `$XDG_*` locations, and the session id is fresh per `init`.

## Adding a key or an action

1. Add the action to the CLI and the engine (`Action` in `engine/types.rs`), and a default shortcut in the config. Check the key does not clash in any of the three shells.
2. Teach `parse_key` the spelling if new, and add its unit tests (`key_translation`, `bad_keys`).
3. Update all three templates. They must behave the same; differences go in the docs.
4. Add a case to `conformance.sh`: send the key with tmux (`send-keys`), read the pane, and assert on the buffer and the mock's `last-request.json`.

## Adding a shell

1. A new template and a generator in `init.rs`; add it to `generate` and to `install-shell` (rc file location, block markers, removal).
2. Add it to `run-all.sh`, the CI package install line (`.github/workflows/ci.yml`), and `doctor`.
3. Document the version floor and any limits in `guides/shells.mdx` and the roadmap.
4. Add `have_tool(...)`-gated syntax tests (see `crates/shellmate/tests/init_syntax.rs`) so the generated glue parses with the shell's own `-n`/`--no-execute`.

## Test

```sh
cargo test --workspace --locked
REQUIRE_TOOLS=1 cargo test -p shellmate --locked    # glue syntax tests need the real shells
tools/e2e/conformance.sh zsh                         # one shell, then run-all.sh
```

Tmux tests depend on timing. Wait for a marker in the pane, never for a fixed time. If a case is flaky, see `.agents/skills/investigate-flaky-test/SKILL.md`.
