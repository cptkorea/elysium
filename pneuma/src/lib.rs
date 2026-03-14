use thiserror::Error;

pub mod dag;
pub mod raft;
pub mod schema;
pub mod scheduler;

#[derive(Debug, Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("yaml parse error: {0}")]
    ParseError(#[from] serde_yml::Error),
    #[error("unknown task \"{0}\" referenced in depends_on")]
    UnknownTask(String),
    #[error("cycle detected involving task \"{0}\"")]
    CycleDetected(String),
}
