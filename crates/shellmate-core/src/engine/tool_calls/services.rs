//! What the core lets a tool use: its own classifier, path checks and process runner, over the machine
//! `Host` describes. A tool reaches the machine only through these, so the sanitized environment, the
//! timeout, the output cap and the checks are the core's, not the tool's.

use crate::command;
use crate::config::ExecEnv;
use crate::ports::Host;
use crate::tools;
use async_trait::async_trait;
use shellmate_api::{
    CommandRisk, FileTarget, FileText, ProcessEnv, ProcessOutput, ProcessSpec, ToolServices,
};
use std::path::PathBuf;

pub(in crate::engine) struct CoreServices<'a> {
    pub(in crate::engine) host: &'a Host,
}

#[async_trait]
impl ToolServices for CoreServices<'_> {
    fn env_var(&self, name: &str) -> Option<String> {
        self.host.env.var(name)
    }

    fn is_dir(&self, path: &str) -> bool {
        self.host.fs.is_dir(std::path::Path::new(path))
    }

    fn parse_command(&self, line: &str) -> Result<Vec<String>, String> {
        command::parse_strict(line).map_err(|why| why.describe())
    }

    fn find_program(&self, name: &str) -> Option<PathBuf> {
        let path = self.host.env.var("PATH").unwrap_or_default();
        command::resolve_program(name, &path)
    }

    fn classify_command(&self, line: &str) -> CommandRisk {
        let a = command::classify(line);
        CommandRisk {
            safe: a.verdict == command::Verdict::Safe,
            risk: a.risk,
            reasons: a.reasons,
        }
    }

    fn command_rule_matches(&self, rule: &str, argv: &[String]) -> bool {
        command::rule_matches(rule, argv)
    }

    async fn run_process(&self, spec: &ProcessSpec) -> ProcessOutput {
        let prepared = command::Prepared {
            program: spec.program.clone(),
            argv: spec.argv.clone(),
            cwd: spec.cwd.clone(),
        };
        let env = match spec.env {
            ProcessEnv::Sanitized => ExecEnv::Sanitized,
            ProcessEnv::Inherit => ExecEnv::Inherit,
        };
        let r = tools::run_exec(
            &prepared,
            spec.timeout,
            spec.max_output,
            env,
            self.host.env.as_ref(),
        )
        .await;
        ProcessOutput {
            exit: r.exit,
            timed_out: r.timed_out,
            stdout: r.stdout,
            stderr: r.stderr,
            truncated: r.truncated,
            elapsed: r.elapsed,
        }
    }

    fn vet_file(
        &self,
        arg: &str,
        cwd: Option<&str>,
        roots: &[String],
    ) -> Result<FileTarget, String> {
        let home = self.host.env.var("HOME");
        let t = tools::prepare_read(arg, cwd, home.as_deref(), roots)?;
        Ok(FileTarget {
            path: t.path,
            shown: t.shown,
            size: t.size,
        })
    }

    fn read_file(&self, target: &FileTarget, max_bytes: usize) -> Result<FileText, String> {
        let t = tools::ReadTarget {
            path: target.path.clone(),
            shown: target.shown.clone(),
            size: target.size,
        };
        let r = tools::read_file(&t, max_bytes)?;
        Ok(FileText {
            text: r.text,
            bytes: r.bytes,
            truncated: r.truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::MapEnv;
    use shellmate_api::Risk;
    use std::time::Duration;

    fn host(path: &str) -> Host {
        Host::system().with_env(MapEnv::new().with("PATH", path).with("HOME", "/home/u"))
    }

    #[test]
    fn the_services_look_at_the_host_not_the_process() {
        let h = host("/usr/bin:/bin");
        let s = CoreServices { host: &h };
        assert_eq!(s.env_var("HOME").as_deref(), Some("/home/u"));
        assert_eq!(s.env_var("NOPE"), None);
        assert!(s.is_dir("/"));
        assert!(!s.is_dir("/no/such/dir"));
        assert!(s.find_program("sh").is_some());
        let none = host("");
        assert_eq!(CoreServices { host: &none }.find_program("sh"), None);
    }

    #[test]
    fn commands_are_parsed_strictly_and_classified() {
        let h = host("/usr/bin:/bin");
        let s = CoreServices { host: &h };
        assert_eq!(s.parse_command("git status").unwrap(), ["git", "status"]);
        assert!(s.parse_command("ls | wc").is_err());
        let safe = s.classify_command("ls -l");
        assert!(safe.safe && safe.risk == Risk::Low && safe.reasons.is_empty());
        let bad = s.classify_command("rm -rf /");
        assert!(!bad.safe && bad.risk == Risk::High && !bad.reasons.is_empty());
        assert!(s.command_rule_matches("git status", &["git".into(), "status".into()]));
        assert!(!s.command_rule_matches("git status", &["git".into(), "push".into()]));
    }

    #[tokio::test]
    async fn a_process_runs_with_a_timeout_and_a_cap() {
        let h = host("/usr/bin:/bin");
        let s = CoreServices { host: &h };
        let spec = |script: &str, timeout_ms: u64, cap: usize| ProcessSpec {
            program: "/bin/sh".into(),
            argv: vec!["sh".into(), "-c".into(), script.into()],
            cwd: None,
            timeout: Duration::from_millis(timeout_ms),
            max_output: cap,
            env: ProcessEnv::Sanitized,
        };
        let ok = s
            .run_process(&spec("echo hi; echo oops >&2; exit 3", 10_000, 100))
            .await;
        assert_eq!(
            (ok.exit, ok.timed_out, ok.stdout.as_str()),
            (Some(3), false, "hi")
        );
        assert_eq!(ok.stderr, "oops");
        let capped = s.run_process(&spec("yes | head -c 5000", 10_000, 10)).await;
        assert!(capped.truncated && capped.stdout.len() <= 10);
        let slow = s.run_process(&spec("exec sleep 30", 200, 100)).await;
        assert!(slow.timed_out);
        // the sanitized environment is the host's allow-list, not the process's
        let env = s.run_process(&spec("echo $HOME", 10_000, 100)).await;
        assert_eq!(env.stdout, "/home/u");
    }

    #[test]
    fn a_file_is_vetted_and_then_read_with_a_cap() {
        let d = std::env::temp_dir().join(format!("sm-svc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.txt"), "0123456789").unwrap();
        std::fs::write(d.join(".env"), "KEY=1").unwrap();
        let h = host("/usr/bin:/bin");
        let s = CoreServices { host: &h };
        let cwd = d.display().to_string();
        let t = s.vet_file("a.txt", Some(&cwd), &[]).unwrap();
        assert_eq!((t.shown.as_str(), t.size), ("a.txt", 10));
        let r = s.read_file(&t, 4).unwrap();
        assert_eq!((r.text.as_str(), r.bytes, r.truncated), ("0123", 10, true));
        assert!(s.vet_file(".env", Some(&cwd), &[]).is_err(), "credentials");
        assert!(
            s.vet_file("/etc/passwd", Some(&cwd), &[]).is_err(),
            "outside the roots"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
