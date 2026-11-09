---
name: add-config-setting
description: Add or change a shellmate config key: strict parsing, defaults, validation, config show/check, setup, docs and changelog. Includes the rules for secrets.
---

# Add a config setting

Config is TOML in `crates/shellmate-core/src/config/` (one file per topic: `sections.rs`, `providers.rs`, `tools.rs`; `validate.rs` holds the cross-checks). Every struct uses `#[serde(deny_unknown_fields)]`, so a typo is an error, not a silent no-op. Keep it that way.

## Rules

- **Secrets are never literal.** A credential setting accepts only `env:NAME`, `file:PATH` or `cmd:PROGRAM ARGS` references, resolved by the host before a plugin sees them (`secrets.rs`). Do not add a field that accepts a raw key. Resolved values are wrapped in `shellmate_api::Secret`.
- **Safe default.** The default must be the stricter, more private, less powerful choice. Features that run things or send more context are off until the user turns them on.
- **Privacy caps still win.** A setting cannot raise the context level above the provider's `max_context_level`.
- **Errors say what to do.** `Error::new("config.<area>", "...").with_hint("...")`. Name the key and the table in the message, for example `plugins."builtin:redact".settings`.
- **No surprise at start-up.** Parsing and validation must stay cheap (the 30 ms budget). No network or process spawning in `check_static`.

## Steps

1. Add the field with a doc comment, a `#[serde(default = "...")]` and a default function. Use a typed value (`u64`, a small enum, `Duration` via the existing parser) rather than a free string.
2. Validate in `check_static` (checks that need no plugin registry) or in the plugin's constructor (checks that need its settings). Reject ranges that make no sense, and warn (not fail) for settings that are legal but risky. `config check` shows warnings. Example: broad `preapproved` rules.
3. Use the value where it matters, and make sure a short internal time limit goes through `limits::scaled`.
4. Show it: `shellmate config show` prints the effective config (secrets as references only). `setup`/`config init` writes a template (`crates/shellmate/src/admin/templates.rs`); add the key there only if a new user needs it. Do not write redundant defaults.
5. If `doctor` could help the user diagnose it, add a check.
6. Docs: a row in `website/src/content/docs/docs/reference/configuration.md` with the default, and the guide that explains the behaviour. Add to the troubleshooting page if misuse is likely. `CHANGELOG.md` under "Unreleased".

## Tests

- Parse: default, valid, wrong type, out of range, unknown key (must fail), and a layered override if layering applies.
- A secret field rejects a literal value and accepts `env:`/`file:`; the error never contains the secret.
- `config check` output for the warning, if there is one.
- Behaviour: the setting changes what the binary does (use `common::run_with` with a mock server and a config string).
- A privacy-relevant setting: a test that the request payload respects it (`shellmate invoke --dry-run` shows what would be sent after redaction, and `crates/shellmate/tests/privacy.rs` has the pattern).

## Done when

fmt, clippy and tests are clean, `shellmate config check` accepts the example from the docs, and the docs table and the code agree on the default (check the source, not memory).
