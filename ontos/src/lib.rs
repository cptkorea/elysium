//! # Ontos — Durable LSM-Tree Storage Engine
//!
//! Ontos is a log-structured merge-tree (LSM-tree) storage engine that
//! persists arbitrary byte key-value pairs to disk. It is designed for
//! high write throughput with durable, crash-recoverable storage.
//!
//! ## Components
//!
//! - [`db::MemTable`] — in-memory sorted buffer for recent writes.
//! - [`db::SSTable`] — immutable on-disk sorted run flushed from a full MemTable.
//! - [`driver::Driver`] — top-level engine that coordinates MemTable, SSTable
//!   flushes, and (in future phases) WAL and merged reads.
//! - [`sorted_store::SortedStore`] — pluggable trait for the MemTable's
//!   backing data structure (BTreeMap, SkipList, etc.).

use thiserror::Error;

pub mod db;
pub mod driver;
#[path = "sorted-store.rs"]
pub mod sorted_store;
pub mod wal;

/// Errors produced by the ontos storage engine.
#[derive(Debug, Error)]
pub enum Error {
    /// A bincode serialization or deserialization operation failed.
    #[error("bincode error")]
    BincodeError,
    /// An underlying I/O operation failed.
    #[error("i/o error")]
    IoError(#[from] std::io::Error),
    /// The active MemTable has reached its configured entry capacity.
    #[error("memtable full")]
    MemTableFull,
}
