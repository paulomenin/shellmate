//! Secret references in config: `env:NAME`, `file:/path`, `cmd:program args` and `keyring:service/account`
//! (`security.md` §4). Inline secrets are refused. `cmd:` is run by `secret_cmd`, and the engine runs it on
//! first use, not at start-up. `keyring:` is sugar: it becomes the `cmd:` line of the system's own keyring
//! tool, so it has the same limits, the same laziness and no new dependency.

use crate::ports::Host;
use crate::secret_cmd::{self, CmdOptions};
use shellmate_api::{Error, Result};
use std::borrow::Cow;

/// Setting names whose values are secrets and must be references.
pub fn is_secret_key(key: &str) -> bool {
    matches!(key, "api_key" | "token" | "password" | "secret")
        || key.ends_with("_key")
        || key.ends_with("_token")
        || key.ends_with("_secret")
}

/// The line after `cmd:`.
pub fn command_line(value: &str) -> Option<&str> {
    value.strip_prefix("cmd:").map(str::trim)
}

/// The `cmd:` line a reference stands for: the line itself for `cmd:`, the keyring tool's line for
/// `keyring:`, and `None` for anything else. A malformed `keyring:` reference is an error.
pub fn lookup_line<'a>(key: &str, value: &'a str) -> Result<Option<Cow<'a, str>>> {
    if let Some(line) = command_line(value) {
        return Ok(Some(Cow::Borrowed(line)));
    }
    match value.strip_prefix("keyring:") {
        Some(reference) => {
            keyring_line(key, reference, std::env::consts::OS).map(|l| Some(l.into()))
        }
        None => Ok(None),
    }
}

/// `service/account` as the command line of the keyring tool of `os` (`std::env::consts::OS`):
/// `secret-tool` (libsecret: GNOME Keyring, KWallet) on Linux, `security` (Keychain) on macOS. Both
/// names are restricted to characters that need no quoting, so nothing can add an argument.
fn keyring_line(key: &str, reference: &str, os: &str) -> Result<String> {
    let bad = |why: &str| {
        Error::new(
            "config.invalid_keyring",
            format!("`{key}`: keyring reference `{reference}` {why}"),
        )
        .with_hint("write `keyring:service/account`, for example `keyring:shellmate/openai`")
    };
    let (service, account) = reference
        .split_once('/')
        .ok_or_else(|| bad("needs a service and an account"))?;
    for part in [service, account] {
        if part.is_empty() {
            return Err(bad("needs a service and an account"));
        }
        let plain = part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._@:+-".contains(c));
        if !plain || part.starts_with('-') {
            return Err(bad(
                "may only use letters, digits and . _ @ : + - (not first)",
            ));
        }
    }
    match os {
        "linux" => Ok(format!(
            "secret-tool lookup service {service} account {account}"
        )),
        "macos" => Ok(format!(
            "security find-generic-password -s {service} -a {account} -w"
        )),
        other => Err(Error::new(
            "config.unsupported",
            format!("`{key}`: keyring references are not supported on {other}"),
        )
        .with_hint("use env:, file: or cmd:")),
    }
}

/// `key=value` as `config show` prints a setting. A `cmd:` secret is a reference, so it is shown, unless a
/// key seems to be written in its arguments; any other value is printed as written (the loader already
/// refused inline secrets).
pub fn show_setting(key: &str, value: &toml::Value, env: &dyn crate::ports::Env) -> String {
    match (is_secret_key(key), value.as_str().and_then(command_line)) {
        (true, Some(line)) => format!("{key}=\"cmd:{}\"", secret_cmd::display(line, env)),
        _ => format!("{key}={value}"),
    }
}

pub fn resolve(key: &str, value: &str, host: &Host) -> Result<String> {
    resolve_with(key, value, host, &CmdOptions::default())
}

/// Like [`resolve`], with the `[secrets]` settings that a `cmd:` reference needs.
pub fn resolve_with(key: &str, value: &str, host: &Host, opts: &CmdOptions) -> Result<String> {
    if let Some(line) = lookup_line(key, value)? {
        return secret_cmd::run(key, &line, host, opts);
    }
    if let Some(name) = value.strip_prefix("env:") {
        return host.env.var(name).ok_or_else(|| {
            Error::new(
                "config.secret_unset",
                format!("`{key}` refers to env var `{name}`, which is not set"),
            )
            .with_hint(format!(
                "export {name}=... in the environment that starts your shell"
            ))
        });
    }
    if let Some(path) = value.strip_prefix("file:") {
        return read_secret_file(key, path, host);
    }
    Err(Error::new(
        "config.inline_secret",
        format!("`{key}` looks like a secret but is not a reference"),
    )
    .with_hint(
        "write `env:NAME`, `file:/path/to/key`, `keyring:service/account` or `cmd:program args` instead of the value itself",
    ))
}

fn read_secret_file(key: &str, path: &str, host: &Host) -> Result<String> {
    let path = expand_home(path, host.env.as_ref());
    let meta = host.fs.metadata(&path).map_err(|e| {
        Error::new(
            "config.secret_unset",
            format!("`{key}`: cannot read {}: {e}", path.display()),
        )
    })?;
    if meta.mode & 0o077 != 0 {
        return Err(Error::new(
            "config.secret_permissions",
            format!("{} is readable by other users", path.display()),
        )
        .with_hint(format!("chmod 600 {}", path.display())));
    }
    let text = host.fs.read_to_string(&path).map_err(|e| {
        Error::new(
            "config.secret_unset",
            format!("`{key}`: cannot read {}: {e}", path.display()),
        )
    })?;
    Ok(text.trim_end_matches(['\n', '\r']).to_string())
}

pub fn expand_home(path: &str, env: &dyn crate::ports::Env) -> std::path::PathBuf {
    match (path.strip_prefix("~/"), env.var("HOME")) {
        (Some(rest), Some(home)) => std::path::Path::new(&home).join(rest),
        _ => path.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{MapEnv, MemFs};

    fn host(env: MapEnv, fs: MemFs) -> Host {
        Host::system().with_env(env).with_fs(fs)
    }

    #[test]
    fn inline_secrets_are_refused() {
        assert_eq!(
            resolve("api_key", "sk-abc", &Host::system())
                .unwrap_err()
                .code,
            "config.inline_secret"
        );
    }

    #[test]
    fn env_references() {
        let h = host(MapEnv::new().with("SM_KEY", "v"), MemFs::new());
        assert_eq!(resolve("api_key", "env:SM_KEY", &h).unwrap(), "v");
        assert_eq!(
            resolve("api_key", "env:SM_UNSET", &h).unwrap_err().code,
            "config.secret_unset"
        );
    }

    #[test]
    fn file_references_require_private_permissions() {
        let open = host(
            MapEnv::new(),
            MemFs::new().file("/k/key", "topsecret\n", 0o644, 1000),
        );
        assert_eq!(
            resolve("api_key", "file:/k/key", &open).unwrap_err().code,
            "config.secret_permissions"
        );
        let private = host(
            MapEnv::new(),
            MemFs::new().file("/k/key", "topsecret\n", 0o600, 1000),
        );
        assert_eq!(
            resolve("api_key", "file:/k/key", &private).unwrap(),
            "topsecret"
        );
        assert_eq!(
            resolve("api_key", "file:/k/missing", &private)
                .unwrap_err()
                .code,
            "config.secret_unset"
        );
    }

    #[test]
    fn a_tilde_means_the_home_directory_of_the_environment() {
        let h = host(
            MapEnv::new().with("HOME", "/home/u"),
            MemFs::new().file("/home/u/.keys/k", "v", 0o600, 1000),
        );
        assert_eq!(resolve("api_key", "file:~/.keys/k", &h).unwrap(), "v");
        assert_eq!(
            expand_home("~/x", &MapEnv::new()),
            std::path::PathBuf::from("~/x")
        );
    }

    #[test]
    fn every_secret_key_shape_is_recognised_and_other_names_are_not() {
        for k in [
            "api_key", "token", "password", "secret", "x_key", "x_token", "x_secret",
        ] {
            assert!(is_secret_key(k), "{k}");
        }
        for k in [
            "model",
            "keyboard",
            "tokens",
            "secrets",
            "endpoint",
            "timeout_secs",
        ] {
            assert!(!is_secret_key(k), "{k}");
        }
    }

    #[test]
    fn config_show_prints_a_cmd_reference_and_hides_a_key_written_in_it() {
        let env = MapEnv::new();
        let v = |s: &str| toml::Value::String(s.into());
        assert_eq!(
            show_setting("api_key", &v("cmd:pass show api/openai"), &env),
            "api_key=\"cmd:pass show api/openai\""
        );
        let hidden = show_setting("api_key", &v("cmd:echo sk-abcdef0123456789abcdef"), &env);
        assert!(
            hidden.starts_with("api_key=\"cmd:echo (arguments hidden"),
            "{hidden}"
        );
        assert!(!hidden.contains("sk-abcdef"));
        // other settings are printed as written, and a `cmd:` value is only special in a secret setting
        assert_eq!(show_setting("model", &v("cmd:x"), &env), "model=\"cmd:x\"");
        assert_eq!(
            show_setting("api_key", &v("env:K"), &env),
            "api_key=\"env:K\""
        );
    }

    #[test]
    fn keyring_references_become_the_tool_of_the_platform() {
        let line = |r, os| keyring_line("api_key", r, os);
        assert_eq!(
            line("shellmate/openai", "linux").unwrap(),
            "secret-tool lookup service shellmate account openai"
        );
        assert_eq!(
            line("shellmate/me@x.org", "macos").unwrap(),
            "security find-generic-password -s shellmate -a me@x.org -w"
        );
        assert_eq!(
            line("shellmate/openai", "windows").unwrap_err().code,
            "config.unsupported"
        );
    }

    #[test]
    fn malformed_keyring_references_are_refused() {
        for bad in [
            "",
            "shellmate",
            "shellmate/",
            "/openai",
            "a b/openai",
            "shellmate/open ai",
            "shellmate/a'b",
            "shellmate/$(x)",
            "-s/openai",
            "shellmate/-w",
            "shellmate/a\nb",
        ] {
            assert_eq!(
                keyring_line("api_key", bad, "linux").unwrap_err().code,
                "config.invalid_keyring",
                "{bad:?}"
            );
        }
        // what a valid reference expands to must split into plain words
        let env = MapEnv::new();
        let l = keyring_line("k", "svc/acct.1", "linux").unwrap();
        assert_eq!(secret_cmd::split("k", &l, &env).unwrap().len(), 6);
    }

    #[test]
    fn lookup_line_covers_both_schemes_and_nothing_else() {
        assert_eq!(
            lookup_line("k", "cmd: pass show x").unwrap().as_deref(),
            Some("pass show x")
        );
        assert!(lookup_line("k", "env:X").unwrap().is_none());
        assert!(lookup_line("k", "file:/x").unwrap().is_none());
        assert_eq!(
            lookup_line("k", "keyring:bad").unwrap_err().code,
            "config.invalid_keyring"
        );
    }

    #[test]
    fn key_names() {
        assert!(
            is_secret_key("api_key") && is_secret_key("proxy_token") && !is_secret_key("model")
        );
    }
}
