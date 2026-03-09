use thiserror::Error;

pub mod db;
pub mod driver;

#[derive(Debug, Error)]
pub enum Error {
    #[error("bincode error")]
    BincodeError,
    #[error("i/o error")]
    IoError(#[from] std::io::Error),
    #[error("memtable full")]
    MemTableFull,
}
