//! What the model is told about the shell: the context for a request, and the project and git sections
//! gathered once per request.

use super::{Engine, Gathered};
use crate::envelope::Envelope;
use crate::prompt::Context;

impl Engine {
    pub(super) fn context(&self, env: &Envelope) -> Context {
        let mut ctx = Context::from_envelope(
            env,
            self.os.clone(),
            self.home().as_deref(),
            self.cfg.context.history_lines,
        );
        // Output the user explicitly captured with `shellmate run` (level 2 and above decides if it is sent).
        {
            let g = self.gathered.lock().expect("gathered lock");
            ctx.project = g.project.clone();
            ctx.git = g.git.as_ref().map(|i| i.section.clone());
            ctx.git_untrusted = g.git.as_ref().is_some_and(|i| i.untrusted);
        }
        ctx.capture = env
            .session
            .as_deref()
            .and_then(|s| crate::capture::load(&self.host, s))
            .map(|c| {
                crate::capture::prompt_section(
                    &c,
                    self.cfg.context.capture_bytes,
                    self.host.clock.unix_secs(),
                )
            });
        ctx
    }

    /// Gather the project and git sections for this request. Runs `git` (bounded by a deadline) off the
    /// async threads, and only when the context level can carry the result.
    pub(super) async fn prefetch(&self, env: &Envelope) {
        let max_level = self
            .providers
            .iter()
            .map(|p| p.max_level)
            .max()
            .unwrap_or(0)
            .min(self.default_level());
        let c = &self.cfg.context;
        let cwd = crate::collect::dir_of(env.cwd.as_deref(), self.host.fs.as_ref());
        let home = self.home();
        let host = self.host.clone();
        let (want_project, want_git, log) = (
            c.project && max_level >= 1,
            c.git && max_level >= 2,
            c.git_log_lines,
        );
        let gathered = match cwd {
            Some(cwd) if want_project || want_git => {
                tokio::task::spawn_blocking(move || Gathered {
                    project: want_project
                        .then(|| {
                            crate::collect::project(
                                &cwd,
                                home.as_deref().map(std::path::Path::new),
                                host.fs.as_ref(),
                            )
                        })
                        .flatten(),
                    git: want_git
                        .then(|| crate::collect::git_info(host.env.as_ref(), &cwd, log))
                        .flatten(),
                })
                .await
                .unwrap_or_default()
            }
            _ => Gathered::default(),
        };
        *self.gathered.lock().expect("gathered lock") = gathered;
    }
}
