use crate::{ProviderError, ProviderInfo};
use async_trait::async_trait;

/// A typed question for a decision model: one forward pass, a label with probabilities, no free text.
#[derive(Debug, Clone, PartialEq)]
pub enum Question {
    /// Pick one label. `criteria` is `(label, description)`, in order.
    Choice {
        instructions: String,
        criteria: Vec<(String, String)>,
    },
    /// A yes/no statement; the answer is the probability that it holds.
    YesNo { instructions: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecisionRequest {
    /// The record the questions are about, as `(field, text)`.
    pub state: Vec<(String, String)>,
    /// `(question id, question)`.
    pub questions: Vec<(String, Question)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Choice {
        label: String,
        /// 0..=1, how far the top label stands out.
        confidence: f32,
        probabilities: Vec<(String, f32)>,
    },
    /// Probability that the statement holds, 0..=1.
    YesNo { probability: f32 },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DecisionResponse {
    /// `(question id, answer)`.
    pub answers: Vec<(String, Answer)>,
}

impl DecisionResponse {
    pub fn get(&self, id: &str) -> Option<&Answer> {
        self.answers.iter().find(|(i, _)| i == id).map(|(_, a)| a)
    }
}

/// A decision-model backend (for example an Ollama or Ollaya `/v1/systemone` endpoint). Configured like a
/// provider, with its own trust level.
#[async_trait]
pub trait Decider: Send + Sync {
    fn info(&self) -> ProviderInfo;
    async fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse, ProviderError>;
}
