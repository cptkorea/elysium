use thiserror::Error;

pub mod scheduler;
pub mod schema;

#[derive(Debug, Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("yaml parse error: {0}")]
    ParseError(#[from] serde_yml::Error),
    #[error("unknown task \"{0}\" referenced in depends_on")]
    UnknownTask(String),
    #[error(transparent)]
    GraphError(#[from] elysium_common::dag::Error),
}
