---
name: add-plugin
description: Bootstrap a new first-party shellmate plugin (provider, decider, llm middleware or conversation store) as a module of `crates/shellmate-builtin`, register it, test it and document it.
---

# Add a plugin

Plugins depend only on `shellmate-api`. If you need something from `shellmate-core`, the design is wrong: extend the API instead, deliberately.

## 1. Pick the kind

| Kind | Trait (in `crates/shellmate-api/src/`) | Register with | Configured under |
|---|---|---|---|
| Provider | `Provider` (`provider.rs`): `info`, `chat`, optional `chat_stream` | `register_provider` | `[providers.<name>]` with `plugin = "builtin:<id>"` and `[providers.<name>.settings]` |
| Decider | `Decider` (`decider.rs`): `info`, `decide` | `register_decider` | `[deciders.<name>]`, same shape as providers |
| `llm` middleware | `Middleware<ChatRequest, ChatStream>` (`middleware.rs`): `id`, `call(request, next)` | `register_llm_middleware(id, role, ctor)` | `[llm] middleware` list and `[plugins."builtin:<id>"]` (`enabled`, `settings`) |
| Store | `Store` (`store.rs`) | `register_store` | the store section of the config |

The nearest existing example for each: `provider_openai_compatible`, `decider_systemone`, `middleware_redact`, `store_tmpfs` (modules of `shellmate-builtin`). Copy the closest one.

A middleware that removes or hides sensitive data should declare `MiddlewareRole::Redactor`. The core uses roles to enforce INV-2. A middleware that only observes uses `Observer`, one that rewrites for non-privacy reasons uses `Filter`. Never give a role you do not fulfil.

## 2. Create the module

All first-party plugins live in one crate, `crates/shellmate-builtin`, for now. Each is a module behind a cargo feature of the same name. The module may use only `shellmate_api` from the workspace, never `shellmate_core`, as an external plugin would.

1. `crates/shellmate-builtin/src/<kind>_<name>/mod.rs`, with one file per concern. Dependencies go in `crates/shellmate-builtin/Cargo.toml` as `optional = true`, and the feature `<kind>-<name>` lists them as `dep:<crate>`. Add the feature to `default`. Dev-dependencies (`shellmate-testing`, `tokio`, `proptest`) are already there.
2. In `src/lib.rs` add `#[cfg(feature = "<kind>-<name>")] pub mod <kind>_<name>;` and a `#[cfg(feature = ...)] <kind>_<name>::register(&mut r);` line in `registry()`.
3. In the module's `mod.rs`:
   ```rust
   pub const PLUGIN_ID: &str = "<name>";            // no `builtin:` prefix; the user writes `builtin:<name>`
   pub fn register(registry: &mut shellmate_api::Registry) {
       registry.register_provider(PLUGIN_ID, Box::new(|settings| Ok(Arc::new(MyProvider::from_settings(settings)?))));
   }
   ```
   Keep `mod.rs` to the id, `register` and the public re-exports. Once the plugin has more than one concern, give each its own file (see `provider_openai_compatible`: `settings`, `presets`, `client`, `wire`, `sse`; and `middleware_redact`: `rules`, `mapping`, `redactor`, `middleware`) and a `tests.rs` for the unit tests. Inside the module, refer to siblings with `super::`. Items shared between your files can be `pub(crate)`; only what plugin users need is `pub`.
4. Parse `settings` (a `serde_json::Value`) strictly. Unknown or wrongly typed keys should give `Error::new("config.<area>", ...).with_hint(...)`. Secrets arrive already resolved as strings from `env:`/`file:` references. Wrap them in `shellmate_api::Secret` so they cannot be printed by accident.
5. Check that the plugin builds alone: `cargo check -p shellmate-builtin --no-default-features --features <kind>-<name>`.

## 3. Behave

- Network endpoints go through `check_endpoint`. Cleartext `http` is accepted only for loopback hosts.
- Errors from a provider or decider use `ProviderError` with the right `ProviderErrorKind`, so the fallback chain can tell a retryable failure from a permanent one.
- Never log or return prompts, answers or secrets in an error message.
- Stay quiet on stdout. The core owns rendering.
- No blocking work in `register` or the constructor. Start-up has a 30 ms budget.

## 4. Test (no model, no network)

- Unit tests for settings parsing: valid, missing, wrongly typed, unknown key, secret handling.
- Behaviour tests with `shellmate-testing`: `FakeProvider`, `MockHttpServer`, `MockSseServer`. For a streaming provider test chunk boundaries (a split in the middle of a multi-byte character, an empty chunk, an error mid-stream).
- For middleware that transforms text, add **property tests**: round trip, idempotence, and independence from how streaming chunks are split. See `crates/shellmate-builtin/tests/middleware_redact_properties.rs` for the pattern (shared `OnceLock` for anything expensive to build).
- Test the refusal paths. For anything touching trust or redaction, show that the unsafe configuration is rejected.
- Add a binary-level test in `crates/shellmate/tests/` if the plugin changes what a user sees (`common::run_with` runs the real binary against a mock server).
- Run `cargo mut -p <crate>` and handle the survivors, if the plugin makes security decisions.

## 5. Wire up and document

- `shellmate config check` and `doctor` should work with the new plugin. Add a `doctor` check if the plugin has an external dependency (reachability).
- Add a row to the plugin table and the source layout table in `website/src/content/docs/docs/concepts/architecture.md`.
- Add or extend a guide in `website/src/content/docs/docs/guides/` and the settings to `reference/configuration.md`. See `.agents/skills/update-docs/SKILL.md`.
- Add a line to `CHANGELOG.md` under "Unreleased".
- Done when: fmt, clippy `-D warnings` and `cargo test --workspace --locked` are clean, the docs site builds, and `tools/bench/perf.py --quick` shows no start-up regression.

External (out-of-process) plugins, manifests and permissions are designed but not built. Do not invent a half version of them.
