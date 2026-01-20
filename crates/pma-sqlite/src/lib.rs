pub mod archive;
pub mod lru;
pub mod store;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PmaSqliteError {
    #[error("sqlite error: {0}")]
    Sqlite(String),
    #[error("pma error: {0}")]
    Pma(#[from] nockvm::pma::PmaError),
    #[error("stack init error: {0}")]
    StackInit(#[from] nockvm::mem::NewStackError),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("archive error: {0}")]
    Archive(String),
    #[error("missing noun id {0}")]
    Missing(i64),
    #[error("invalid sqlite path")]
    InvalidPath,
}

pub type Result<T> = std::result::Result<T, PmaSqliteError>;

pub use archive::ArchivedNoun;
pub use store::{CachedArchive, SqlitePma, SqlitePmaConfig, SqlitePmaStats};

impl From<nockvm::interpreter::Error> for PmaSqliteError {
    fn from(err: nockvm::interpreter::Error) -> Self {
        Self::Serialization(format!("{err:?}"))
    }
}
