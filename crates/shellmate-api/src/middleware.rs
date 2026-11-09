use crate::Result;
use async_trait::async_trait;
use std::sync::Arc;

/// The innermost handler of a stage (e.g. the provider call for the `llm` stage).
#[async_trait]
pub trait Terminal<Q: Send + 'static, R: Send + 'static>: Send + Sync {
    async fn call(&self, request: Q) -> Result<R>;
}

/// Onion-style middleware: it may change the request, call `next` (zero, one or many times)
/// and change the response (`requirements.md`, "Pipeline & Middleware").
#[async_trait]
pub trait Middleware<Q: Send + 'static, R: Send + 'static>: Send + Sync {
    fn id(&self) -> &str;
    async fn call(&self, request: Q, next: Next<'_, Q, R>) -> Result<R>;
}

/// The rest of the chain, handed to a middleware.
pub struct Next<'a, Q: Send + 'static, R: Send + 'static> {
    rest: &'a [Arc<dyn Middleware<Q, R>>],
    terminal: &'a dyn Terminal<Q, R>,
}

impl<'a, Q: Send + 'static, R: Send + 'static> Next<'a, Q, R> {
    /// Start a chain: `layers` are ordered outermost first.
    pub fn new(layers: &'a [Arc<dyn Middleware<Q, R>>], terminal: &'a dyn Terminal<Q, R>) -> Self {
        Self {
            rest: layers,
            terminal,
        }
    }

    pub async fn run(self, request: Q) -> Result<R> {
        match self.rest.split_first() {
            Some((first, rest)) => {
                first
                    .call(
                        request,
                        Next {
                            rest,
                            terminal: self.terminal,
                        },
                    )
                    .await
            }
            None => self.terminal.call(request).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Tag(&'static str, Arc<Mutex<Vec<String>>>);

    #[async_trait]
    impl Middleware<String, String> for Tag {
        fn id(&self) -> &str {
            self.0
        }
        async fn call(&self, request: String, next: Next<'_, String, String>) -> Result<String> {
            self.1.lock().unwrap().push(format!("{} in", self.0));
            let out = next.run(format!("{request}>{}", self.0)).await?;
            self.1.lock().unwrap().push(format!("{} out", self.0));
            Ok(format!("{out}<{}", self.0))
        }
    }

    struct Echo;
    #[async_trait]
    impl Terminal<String, String> for Echo {
        async fn call(&self, request: String) -> Result<String> {
            Ok(format!("[{request}]"))
        }
    }

    #[tokio::test]
    async fn runs_outermost_first_and_unwinds_in_reverse() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let layers: Vec<Arc<dyn Middleware<String, String>>> = vec![
            Arc::new(Tag("a", log.clone())),
            Arc::new(Tag("b", log.clone())),
        ];
        let out = Next::new(&layers, &Echo).run("req".into()).await.unwrap();
        assert_eq!(out, "[req>a>b]<b<a");
        assert_eq!(*log.lock().unwrap(), ["a in", "b in", "b out", "a out"]);
    }

    #[tokio::test]
    async fn empty_chain_calls_terminal() {
        let layers: Vec<Arc<dyn Middleware<String, String>>> = Vec::new();
        assert_eq!(
            Next::new(&layers, &Echo).run("x".into()).await.unwrap(),
            "[x]"
        );
    }
}
