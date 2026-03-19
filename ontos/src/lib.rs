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

use std::time::Duration;

use thiserror::Error;

pub mod db;
pub mod driver;
#[path = "sorted-store.rs"]
pub mod sorted_store;
pub mod wal;

/// Controls when the storage engine calls `fsync` to flush data from OS
/// buffers to stable storage.
///
/// The three modes offer different tradeoffs between write latency,
/// throughput, and crash safety:
///
/// | Mode | Latency | Crash window |
/// |------|---------|--------------|
/// | [`Sync`](Self::Sync) | Highest | None — every ack'd write is durable |
/// | [`Async`](Self::Async) | Low | Up to one flush interval |
/// | [`Volatile`](Self::Volatile) | Lowest | Unbounded — at the OS's discretion |
#[derive(Debug, Clone)]
pub enum DurabilityMode {
    /// Fsync after every WAL append and SSTable write.
    ///
    /// Safest mode — an acknowledged write is guaranteed to survive
    /// power loss. Highest per-write latency because each write blocks
    /// on a round-trip to the storage device.
    Sync,
    /// No per-write fsync. A background thread fsyncs the WAL file at
    /// the given interval.
    ///
    /// A crash can lose at most one interval's worth of acknowledged
    /// writes. Offers a good balance of throughput and durability — for
    /// example, `Async(Duration::from_millis(200))` limits data loss to
    /// the last 200 ms.
    Async(Duration),
    /// No fsync at all. The OS flushes to disk on its own schedule
    /// (typically within a few seconds).
    ///
    /// Fastest mode, but a crash can lose an unbounded amount of recent
    /// writes. Suitable for caches, ephemeral data, or testing.
    Volatile,
}

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
