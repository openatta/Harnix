//! A gate on what is said to a session, before the model sees it.

/// Asked about every message to a session — one that opens a turn and one steered into a running
/// turn alike.
#[async_trait::async_trait]
pub trait PromptGate: Send + Sync {
    async fn inspect(&self, message: &PromptFacts) -> PromptDecision;
}

/// The message a gate is asked about.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PromptFacts {
    session_id: String,
    turn_id: String,
    content: String,
    delivery: Delivery,
}

impl PromptFacts {
    pub fn new(session_id: &str, turn_id: &str, content: &str, delivery: Delivery) -> Self {
        Self {
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            content: content.to_string(),
            delivery,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    /// The message's text.
    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn delivery(&self) -> Delivery {
        self.delivery
    }
}

/// How a message reaches the session.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// It opens a turn.
    Turn,
    /// It joins the running turn.
    Steer,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptDecision {
    Proceed,
    /// Refused, with what the person is told: a turn it would open ends before any model call; a
    /// steered message is left out and the turn goes on.
    Block(String),
}
