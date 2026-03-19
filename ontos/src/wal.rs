//! # Write-Ahead Log (WAL)
//!
//! The WAL provides crash recovery for the [`MemTable`](crate::db::MemTable).
//! Every mutation is serialized and appended to a sequential log file on
//! disk *before* it is applied to the in-memory MemTable. On startup after
//! a crash, the WAL is replayed to reconstruct the MemTable to its
//! pre-crash state.
//!
//! ## Record Format
//!
//! Each record is written as:
//!
//! ```text
//! [4 bytes: payload length (little-endian u32)]
//! [N bytes: bincode-serialized WalRecord]
//! ```
//!
//! This length-prefixed framing allows the recovery path to read records
//! one at a time without needing delimiters or escape sequences.
//!
//! ## Durability
//!
//! The fsync behavior is controlled by [`DurabilityMode`]:
//!
//! - **Sync**: `sync_data()` is called after every append — strongest
//!   guarantee but highest latency per write.
//! - **Async**: writes go to the OS buffer immediately; a background
//!   thread calls `sync_data()` at a fixed interval.
//! - **Volatile**: no fsync at all — the OS flushes when it chooses.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};

use serde::{Deserialize, Serialize};

use crate::{DurabilityMode, Error};

/// A single mutation record stored in the WAL.
///
/// Each [`WalRecord`] maps to one MemTable operation:
///
/// - `Put` stores a key-value pair (or overwrites an existing key).
/// - `Delete` stores a tombstone for a key, marking it as removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WalRecord {
    /// Insert or overwrite a key-value pair.
    Put { key: Vec<u8>, value: Vec<u8> },
    /// Mark a key as deleted (tombstone).
    Delete { key: Vec<u8> },
}

/// State for the background flusher thread used in [`DurabilityMode::Async`].
struct AsyncFlusher {
    /// Shared file handle that the flusher thread calls `sync_data()` on.
    /// Swapped to a new clone on [`Wal::rotate`].
    shared_file: Arc<Mutex<File>>,
    /// Set to `true` to signal the background thread to exit.
    shutdown: Arc<AtomicBool>,
    /// Handle to the background thread, joined on drop.
    handle: Option<JoinHandle<()>>,
}

/// An append-only write-ahead log backed by a single file on disk.
///
/// The WAL is the first line of defense for durability. The typical
/// lifecycle is:
///
/// 1. **Open** or **create** the WAL file via [`Wal::open`].
/// 2. **Append** records with [`Wal::append`].
/// 3. When the MemTable is flushed to an SSTable, **rotate** the WAL
///    via [`Wal::rotate`] to start a fresh log file.
/// 4. On startup, call [`Wal::recover`] to replay the log and
///    reconstruct the MemTable.
///
/// The fsync behavior depends on the [`DurabilityMode`] passed to
/// [`open`](Self::open). In [`Async`](DurabilityMode::Async) mode, a
/// background thread periodically fsyncs; it is shut down automatically
/// when the `Wal` is dropped.
pub struct Wal {
    file: File,
    path: PathBuf,
    durability: DurabilityMode,
    flusher: Option<AsyncFlusher>,
}

impl Wal {
    /// Opens an existing WAL file or creates a new one at `path` with
    /// the given [`DurabilityMode`].
    ///
    /// In [`Async`](DurabilityMode::Async) mode, a background thread is
    /// spawned that periodically fsyncs the WAL file at the configured
    /// interval.
    pub fn open(path: impl Into<PathBuf>, durability: DurabilityMode) -> Result<Self, Error> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;

        let flusher = if let DurabilityMode::Async(interval) = durability {
            Some(spawn_flusher(&file, interval)?)
        } else {
            None
        };

        Ok(Self {
            file,
            path,
            durability,
            flusher,
        })
    }

    /// Serializes a [`WalRecord`] and appends it to the log.
    ///
    /// The record is written as a 4-byte little-endian length prefix
    /// followed by the bincode payload.
    ///
    /// In [`Sync`](DurabilityMode::Sync) mode, `sync_data()` is called
    /// before returning. In [`Async`](DurabilityMode::Async) and
    /// [`Volatile`](DurabilityMode::Volatile) modes, the write goes to
    /// the OS buffer without an immediate fsync.
    pub fn append(&mut self, record: &WalRecord) -> Result<(), Error> {
        let payload = bincode::serialize(record).map_err(|_| Error::BincodeError)?;
        let len = (payload.len() as u32).to_le_bytes();
        self.file.write_all(&len)?;
        self.file.write_all(&payload)?;

        if matches!(self.durability, DurabilityMode::Sync) {
            self.file.sync_data()?;
        }

        Ok(())
    }

    /// Replays the WAL file at `path` and returns all valid records.
    ///
    /// Records are read sequentially from the start of the file. If the
    /// file ends with a partially-written record (e.g. due to a crash
    /// mid-write), the incomplete tail is silently discarded — all fully
    /// written records before it are still returned.
    ///
    /// Returns an empty `Vec` if the file does not exist.
    pub fn recover(path: impl AsRef<Path>) -> Result<Vec<WalRecord>, Error> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut records = Vec::new();

        // Read length-prefixed records until EOF or a partial/corrupt
        // record is encountered. Each record is [4-byte LE length][payload].
        // An incomplete tail (crash mid-write) is silently discarded.
        loop {
            let mut len_buf = [0u8; 4];
            if let Err(e) = reader.read_exact(&mut len_buf) {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    eprintln!("Unexpected EOF while reading length prefix");
                    break;
                }
                return Err(Error::IoError(e));
            }

            let len = u32::from_le_bytes(len_buf) as usize;
            let mut payload = vec![0u8; len];

            if let Err(e) = reader.read_exact(&mut payload) {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    eprintln!("Unexpected EOF while reading payload");
                    break;
                }
                return Err(Error::IoError(e));
            }

            match bincode::deserialize::<WalRecord>(&payload) {
                Ok(record) => records.push(record),
                Err(_) => {
                    eprintln!("Error deserializing WAL payload from bytes");
                    break;
                }
            }
        }

        Ok(records)
    }

    /// Rotates the WAL by truncating the current file to zero length.
    ///
    /// Called after the MemTable has been successfully flushed to an
    /// SSTable on disk. Once the SSTable is durable, the WAL records
    /// that produced it are no longer needed for recovery.
    ///
    /// In [`Async`](DurabilityMode::Async) mode, the background flusher's
    /// file handle is updated to the new file descriptor.
    pub fn rotate(&mut self) -> Result<(), Error> {
        self.file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)?;

        if matches!(self.durability, DurabilityMode::Sync) {
            self.file.sync_data()?;
        }

        if let Some(flusher) = &self.flusher {
            let new_clone = self.file.try_clone()?;
            let mut shared = flusher.shared_file.lock().unwrap();
            *shared = new_clone;
        }

        Ok(())
    }
}

impl Drop for Wal {
    fn drop(&mut self) {
        if let Some(flusher) = self.flusher.take() {
            flusher.shutdown.store(true, Ordering::Relaxed);
            if let Some(handle) = flusher.handle {
                let _ = handle.join();
            }
        }
    }
}

/// Spawns the background flusher thread for [`DurabilityMode::Async`].
///
/// The thread holds a cloned file descriptor and calls `sync_data()`
/// at the given interval. It checks the shutdown flag each iteration
/// and exits when signaled.
fn spawn_flusher(file: &File, interval: std::time::Duration) -> Result<AsyncFlusher, Error> {
    // std::fs::File is not clone, use try_clone which under the hood uses the dup or dup2
    // syscall creating a new file descriptor that points to the same kernel file object
    // removing the need to copy data or allocate memory
    let cloned = file.try_clone()?;
    let shared_file = Arc::new(Mutex::new(cloned));
    let shutdown = Arc::new(AtomicBool::new(false));

    let thread_file = Arc::clone(&shared_file);
    let thread_shutdown = Arc::clone(&shutdown);

    let handle = thread::spawn(move || {
        while !thread_shutdown.load(Ordering::Relaxed) {
            thread::sleep(interval);
            if thread_shutdown.load(Ordering::Relaxed) {
                break;
            }

            let _ = thread_file
                .lock()
                .expect("[spawn_flusher] shared file lock has been poisoned")
                .sync_data();
        }
    });

    Ok(AsyncFlusher {
        shared_file,
        shutdown,
        handle: Some(handle),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn append_and_recover() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.log");

        {
            let mut wal = Wal::open(&path, DurabilityMode::Sync).unwrap();
            wal.append(&WalRecord::Put {
                key: b"k1".to_vec(),
                value: b"v1".to_vec(),
            })
            .unwrap();
            wal.append(&WalRecord::Delete {
                key: b"k2".to_vec(),
            })
            .unwrap();
            wal.append(&WalRecord::Put {
                key: b"k3".to_vec(),
                value: b"v3".to_vec(),
            })
            .unwrap();
        }

        let records = Wal::recover(&path).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(
            records[0],
            WalRecord::Put {
                key: b"k1".to_vec(),
                value: b"v1".to_vec()
            }
        );
        assert_eq!(
            records[1],
            WalRecord::Delete {
                key: b"k2".to_vec()
            }
        );
        assert_eq!(
            records[2],
            WalRecord::Put {
                key: b"k3".to_vec(),
                value: b"v3".to_vec()
            }
        );
    }

    #[test]
    fn recover_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.wal");

        let records = Wal::recover(&path).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn recover_truncated_record() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.log");

        // Drop the WAL to close the file, simulating a process that
        // wrote one good record before crashing.
        {
            let mut wal = Wal::open(&path, DurabilityMode::Sync).unwrap();
            wal.append(&WalRecord::Put {
                key: b"good".to_vec(),
                value: b"val".to_vec(),
            })
            .unwrap();
        }

        // Append garbage bytes that look like a partial record.
        {
            let mut file = OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(&100u32.to_le_bytes()).unwrap();
            file.write_all(b"short").unwrap();
        }

        let records = Wal::recover(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0],
            WalRecord::Put {
                key: b"good".to_vec(),
                value: b"val".to_vec()
            }
        );
    }

    #[test]
    fn rotate_clears_log() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.log");

        let mut wal = Wal::open(&path, DurabilityMode::Sync).unwrap();
        wal.append(&WalRecord::Put {
            key: b"k".to_vec(),
            value: b"v".to_vec(),
        })
        .unwrap();

        wal.rotate().unwrap();

        let records = Wal::recover(&path).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn recover_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.log");

        Wal::open(&path, DurabilityMode::Sync).unwrap();

        let records = Wal::recover(&path).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn recover_truncated_length_prefix() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.log");

        // Drop the WAL to close the file, simulating a process that
        // wrote one good record before crashing.
        {
            let mut wal = Wal::open(&path, DurabilityMode::Sync).unwrap();
            wal.append(&WalRecord::Put {
                key: b"good".to_vec(),
                value: b"val".to_vec(),
            })
            .unwrap();
        }

        // Append only 2 of the 4 length-prefix bytes.
        {
            let mut file = OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(&[0x10, 0x00]).unwrap();
        }

        let records = Wal::recover(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0],
            WalRecord::Put {
                key: b"good".to_vec(),
                value: b"val".to_vec()
            }
        );
    }

    #[test]
    fn recover_corrupt_payload() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.log");

        // Drop the WAL to close the file, simulating a process that
        // wrote one good record before crashing.
        {
            let mut wal = Wal::open(&path, DurabilityMode::Sync).unwrap();
            wal.append(&WalRecord::Put {
                key: b"good".to_vec(),
                value: b"val".to_vec(),
            })
            .unwrap();
        }

        // Write a valid length prefix followed by garbage that bincode
        // cannot deserialize.
        {
            let garbage = b"this is not valid bincode data!!";
            let mut file = OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(&(garbage.len() as u32).to_le_bytes())
                .unwrap();
            file.write_all(garbage).unwrap();
        }

        let records = Wal::recover(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0],
            WalRecord::Put {
                key: b"good".to_vec(),
                value: b"val".to_vec()
            }
        );
    }

    #[test]
    fn append_after_rotate() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.log");

        let mut wal = Wal::open(&path, DurabilityMode::Sync).unwrap();

        wal.append(&WalRecord::Put {
            key: b"before".to_vec(),
            value: b"1".to_vec(),
        })
        .unwrap();

        wal.rotate().unwrap();

        wal.append(&WalRecord::Put {
            key: b"after".to_vec(),
            value: b"2".to_vec(),
        })
        .unwrap();

        let records = Wal::recover(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0],
            WalRecord::Put {
                key: b"after".to_vec(),
                value: b"2".to_vec()
            }
        );
    }

    #[test]
    fn large_value() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.log");
        let big_value = vec![0xAB; 1_000_000];

        {
            let mut wal = Wal::open(&path, DurabilityMode::Sync).unwrap();
            wal.append(&WalRecord::Put {
                key: b"big".to_vec(),
                value: big_value.clone(),
            })
            .unwrap();
        }

        let records = Wal::recover(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0],
            WalRecord::Put {
                key: b"big".to_vec(),
                value: big_value,
            }
        );
    }

    #[test]
    fn volatile_mode_skips_fsync() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.log");

        {
            let mut wal = Wal::open(&path, DurabilityMode::Volatile).unwrap();
            wal.append(&WalRecord::Put {
                key: b"k".to_vec(),
                value: b"v".to_vec(),
            })
            .unwrap();
        }

        let records = Wal::recover(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0],
            WalRecord::Put {
                key: b"k".to_vec(),
                value: b"v".to_vec()
            }
        );
    }

    #[test]
    fn async_mode_flushes_periodically() {
        use std::time::Duration;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.log");

        {
            let mut wal =
                Wal::open(&path, DurabilityMode::Async(Duration::from_millis(50))).unwrap();
            wal.append(&WalRecord::Put {
                key: b"k".to_vec(),
                value: b"v".to_vec(),
            })
            .unwrap();

            // Wait for the background flusher to run at least once.
            thread::sleep(Duration::from_millis(100));
        }

        let records = Wal::recover(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0],
            WalRecord::Put {
                key: b"k".to_vec(),
                value: b"v".to_vec()
            }
        );
    }

    #[test]
    fn async_mode_shutdown_on_drop() {
        use std::time::Duration;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.log");

        // Create and immediately drop — the background thread should
        // shut down cleanly without hanging.
        let wal = Wal::open(&path, DurabilityMode::Async(Duration::from_millis(50))).unwrap();
        drop(wal);
    }

    #[test]
    fn async_mode_rotate_updates_flusher() {
        use std::time::Duration;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.log");

        let mut wal = Wal::open(&path, DurabilityMode::Async(Duration::from_millis(50))).unwrap();

        wal.append(&WalRecord::Put {
            key: b"before".to_vec(),
            value: b"1".to_vec(),
        })
        .unwrap();

        wal.rotate().unwrap();

        wal.append(&WalRecord::Put {
            key: b"after".to_vec(),
            value: b"2".to_vec(),
        })
        .unwrap();

        thread::sleep(Duration::from_millis(100));

        let records = Wal::recover(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0],
            WalRecord::Put {
                key: b"after".to_vec(),
                value: b"2".to_vec()
            }
        );
    }
}
