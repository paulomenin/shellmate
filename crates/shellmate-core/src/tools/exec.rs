//! Running a command for the model: no shell, a sanitized environment, a timeout, capped output.

use super::text::clean_man;
use crate::ports::Env;
use shellmate_api::ProcessOutput;
use std::time::Duration;

/// A small, fixed environment: nothing from the user's secrets, no pager, no colors. Only the variables in
/// the allow list are copied from `vars`.
fn sanitized_env(cmd: &mut tokio::process::Command, vars: &dyn Env) {
    cmd.env_clear();
    for key in [
        "PATH", "HOME", "USER", "LOGNAME", "LANG", "LC_ALL", "LC_CTYPE", "TZ", "TMPDIR",
    ] {
        if let Some(v) = vars.var(key) {
            cmd.env(key, v);
        }
    }
    cmd.env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat")
        .env("SYSTEMD_PAGER", "")
        .env("GIT_TERMINAL_PROMPT", "0");
}

/// Owns the process group of a running command. Dropping it kills the group, so the descendants of a command
/// do not outlive the future that was running it, whether that future finished, timed out or was cancelled.
/// (`kill_on_drop` only kills the immediate child.)
struct GroupGuard(Option<libc::pid_t>);

impl GroupGuard {
    fn kill(&self) {
        if let Some(p) = self.0 {
            // SAFETY: kill has no memory preconditions; a negative pid names the process group.
            unsafe { libc::kill(-p, libc::SIGKILL) };
        }
    }

    /// Call once the leader has been reaped: its group id may be reused from then on, so it must not be signalled.
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Run `prepared` with `execve` on its argv (never through a shell), in its own process group, with no
/// stdin, a timeout and capped output. The group is killed on timeout and when this future is dropped.
///
/// With `ProcessEnv::Sanitized` the child gets the allow-listed variables of `vars`. With `ProcessEnv::Inherit` it
/// gets the real process environment, whatever `vars` says: that mode exists to pass everything on.
pub async fn run_exec(
    prepared: &crate::command::Prepared,
    timeout: Duration,
    max_output: usize,
    env: shellmate_api::ProcessEnv,
    vars: &dyn Env,
) -> ProcessOutput {
    run_exec_with(prepared, timeout, max_output, env, vars, &[], &[]).await
}

/// [`run_exec`] with variables set on top of the environment it picks, and then others removed.
pub async fn run_exec_with(
    prepared: &crate::command::Prepared,
    timeout: Duration,
    max_output: usize,
    env: shellmate_api::ProcessEnv,
    vars: &dyn Env,
    set_env: &[(String, String)],
    remove_env: &[String],
) -> ProcessOutput {
    use tokio::io::AsyncReadExt;
    use tokio::process::Command;
    let started = std::time::Instant::now();
    let failed = |msg: String| ProcessOutput {
        exit: None,
        timed_out: false,
        stdout: String::new(),
        stderr: msg,
        truncated: false,
        elapsed: started.elapsed(),
    };
    let mut cmd = Command::new(&prepared.program);
    cmd.arg0(&prepared.argv[0]).args(&prepared.argv[1..]);
    if let Some(cwd) = &prepared.cwd {
        cmd.current_dir(cwd);
    }
    if env == shellmate_api::ProcessEnv::Sanitized {
        sanitized_env(&mut cmd, vars);
    }
    // A repository's own config can name a program for git to run when it checks the working tree
    // (`core.fsmonitor`), so a plain `git status` in a cloned repository could execute it. Command-scope
    // config overrides the repository's, and the command that is shown to the user stays the same.
    cmd.env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.fsmonitor")
        .env("GIT_CONFIG_VALUE_0", "false");
    for (k, v) in set_env {
        cmd.env(k, v);
    }
    for k in remove_env {
        cmd.env_remove(k);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return failed(format!("could not start: {e}")),
    };
    // declared after `child`, so it is dropped first: the group is killed before the child is reaped
    let mut group = GroupGuard(child.id().map(|p| p as libc::pid_t));
    let mut out = child.stdout.take().expect("piped");
    let mut err = child.stderr.take().expect("piped");
    let cap = max_output as u64;
    let read_capped = |mut r: Box<dyn tokio::io::AsyncRead + Unpin + Send>| async move {
        let mut buf = Vec::new();
        let _ = (&mut r).take(cap + 1).read_to_end(&mut buf).await;
        buf
    };
    let work = async {
        let (o, e) = tokio::join!(
            read_capped(Box::new(&mut out) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
            read_capped(Box::new(&mut err) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)
        );
        // Output is complete (or capped): stop whatever is left, then collect the exit status.
        let status = tokio::time::timeout(Duration::from_millis(500), child.wait()).await;
        (o, e, status)
    };
    let (o, e, status, timed_out) = match tokio::time::timeout(timeout, work).await {
        Ok((o, e, Ok(Ok(st)))) => (o, e, Some(st), false),
        Ok((o, e, _)) => (o, e, None, false),
        Err(_) => (Vec::new(), Vec::new(), None, true),
    };
    group.kill();
    let _ = child.wait().await;
    group.disarm();
    let truncated = o.len() as u64 > cap || e.len() as u64 > cap;
    let text = |mut b: Vec<u8>| {
        b.truncate(max_output);
        clean_man(&String::from_utf8_lossy(&b))
    };
    ProcessOutput {
        exit: status.and_then(|s| s.code()),
        timed_out,
        stdout: text(o),
        stderr: text(e),
        truncated,
        elapsed: started.elapsed(),
    }
}
