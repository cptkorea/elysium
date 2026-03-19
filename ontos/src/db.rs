use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub use crate::sorted_store::SortedStore;
use crate::Error;
use serde::{Deserialize, Serialize};

const DEFAULT_CAPACITY: usize = 10_000;

/// A single key-value record stored in an [`SSTable`].
///
/// The `value` field is `Option<Vec<u8>>` to support tombstones: a `None`
/// value marks the key as deleted. During reads, a tombstone shadows any
/// older live entry for the same key in lower SSTable levels.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Entry {
    pub key: Vec<u8>,
    pub value: Option<Vec<u8>>,
}

/// An in-memory sorted buffer for recent writes.
///
/// `MemTable` accepts writes until it reaches its configured `capacity`,
/// at which point the [`Driver`](crate::driver::Driver) flushes it to an
/// [`SSTable`] on disk and resets the buffer.
///
/// The backing data structure is pluggable via the [`SortedStore`] trait —
/// the default is a [`BTreeMap`], but a [`SkipList`](elysium_common::skiplist::SkipList)
/// can be used instead.
///
/// Values are `Option<Vec<u8>>`: a `Put` stores `Some(bytes)`, while a
/// `Delete` stores `None` (tombstone). This lets tombstones propagate
/// through the flush/merge pipeline unchanged.
pub struct MemTable<S: SortedStore<Vec<u8>, Option<Vec<u8>>> = BTreeMap<Vec<u8>, Option<Vec<u8>>>> {
    items: S,
    size: usize,
    capacity: usize,
}

impl MemTable<BTreeMap<Vec<u8>, Option<Vec<u8>>>> {
    /// Creates a new `MemTable` backed by a [`BTreeMap`] with the
    /// [`DEFAULT_CAPACITY`] of 10,000 entries.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            items: BTreeMap::new(),
            size: 0,
            capacity,
        }
    }
}

impl<S: SortedStore<Vec<u8>, Option<Vec<u8>>>> MemTable<S> {
    /// Creates a new `MemTable` with a caller-supplied [`SortedStore`]
    /// implementation and the given entry capacity.
    pub fn with_store(capacity: usize, store: S) -> Self {
        Self {
            items: store,
            size: 0,
            capacity,
        }
    }

    /// Inserts or overwrites a key-value pair.
    ///
    /// To record a deletion, pass `None` as the value — this inserts a
    /// tombstone that will shadow older entries during reads.
    pub fn write(&mut self, key: Vec<u8>, value: Option<Vec<u8>>) {
        self.items.insert(key, value);
        self.size += 1;
    }

    /// Looks up a key and returns a reference to its value, or `None` if
    /// the key is absent from this MemTable.
    ///
    /// Note: a return of `Some(&None)` means the key has a tombstone — it
    /// was explicitly deleted. A return of `None` means the key was never
    /// written to this MemTable.
    pub fn read<Q>(&self, key: &Q) -> Option<&Option<Vec<u8>>>
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.items.get(key)
    }

    /// Returns `true` when the number of writes has reached the configured
    /// capacity, signaling that the MemTable should be flushed to disk.
    pub fn at_capacity(&self) -> bool {
        self.size >= self.capacity
    }

    /// Removes all entries and resets the write counter to zero.
    pub fn clear(&mut self) {
        self.items.clear();
        self.size = 0;
    }

    /// Drains all entries in sorted key order as [`Entry`] records.
    ///
    /// Used by the [`Driver`](crate::driver::Driver) when flushing the
    /// MemTable into an [`SSTable`].
    pub fn entries(&self) -> Vec<Entry> {
        self.items
            .iter_sorted()
            .map(|(k, v)| Entry {
                key: k.clone(),
                value: v.clone(),
            })
            .collect()
    }
}

/// An immutable, sorted run of [`Entry`] records serialized to disk.
///
/// SSTables are created by flushing a full [`MemTable`] via the
/// [`Driver`](crate::driver::Driver). Each SSTable's entries are sorted
/// by key, enabling binary search for point lookups and efficient range
/// scans.
///
/// Tombstone entries (`value: None`) are preserved in the SSTable so
/// that compaction and merged reads can correctly shadow older values.
#[derive(Deserialize, Serialize)]
pub struct SSTable {
    pub(crate) entries: Vec<Entry>,
}

impl<S: SortedStore<Vec<u8>, Option<Vec<u8>>>> From<&MemTable<S>> for SSTable {
    fn from(memtable: &MemTable<S>) -> Self {
        SSTable {
            entries: memtable.entries(),
        }
    }
}

impl SSTable {
    /// Serializes this SSTable into a byte vector using bincode.
    ///
    /// The resulting bytes are suitable for writing directly to a `.sst`
    /// file on disk.
    pub fn into_bytes(&self) -> Result<Vec<u8>, Error> {
        bincode::serialize(self).map_err(|_| Error::BincodeError)
    }

    /// Deserializes an SSTable from raw bytes (the inverse of [`into_bytes`](Self::into_bytes)).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        bincode::deserialize(bytes).map_err(|_| Error::BincodeError)
    }

    /// Reads and deserializes an SSTable from a `.sst` file on disk.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Error> {
        let mut file = File::open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes)
    }

    /// Looks up a single key using binary search over the sorted entries.
    ///
    /// Returns the matching [`Entry`] if found (which may be a tombstone),
    /// or `None` if the key is not present in this SSTable.
    pub fn get(&self, key: &[u8]) -> Option<&Entry> {
        self.entries
            .binary_search_by(|e| e.key.as_slice().cmp(key))
            .ok()
            .map(|idx| &self.entries[idx])
    }

    /// Returns all entries whose key starts with `prefix`, in sorted order.
    ///
    /// Uses binary search to find the first matching key, then scans
    /// forward while the prefix holds. Tombstone entries are included
    /// in the result so the caller can handle them appropriately.
    pub fn scan(&self, prefix: &[u8]) -> Vec<&Entry> {
        let start = self.entries.partition_point(|e| e.key.as_slice() < prefix);

        self.entries[start..]
            .iter()
            .take_while(|e| e.key.starts_with(prefix))
            .collect()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn put(m: &mut MemTable, key: &str, value: &[u8]) {
        m.write(key.as_bytes().to_vec(), Some(value.to_vec()));
    }

    fn delete(m: &mut MemTable, key: &str) {
        m.write(key.as_bytes().to_vec(), None);
    }

    #[test]
    fn simple_read_write() {
        let mut m = MemTable::new();
        put(&mut m, "apple", b"1");
        put(&mut m, "banana", b"2");
        put(&mut m, "cactus", b"3");

        assert_eq!(m.read("apple".as_bytes()), Some(&Some(b"1".to_vec())));
        assert_eq!(m.read("banana".as_bytes()), Some(&Some(b"2".to_vec())));
        assert_eq!(m.read("cactus".as_bytes()), Some(&Some(b"3".to_vec())));
        assert_eq!(m.read("dummy".as_bytes()), None);

        put(&mut m, "apple", b"5");
        assert_eq!(m.read("apple".as_bytes()), Some(&Some(b"5".to_vec())));
    }

    #[test]
    fn tombstone() {
        let mut m = MemTable::new();
        put(&mut m, "apple", b"1");
        delete(&mut m, "apple");

        // Key exists but is a tombstone.
        assert_eq!(m.read("apple".as_bytes()), Some(&None));
    }

    #[test]
    fn entries_sorted() {
        let mut m = MemTable::new();
        put(&mut m, "cherry", b"3");
        put(&mut m, "apple", b"1");
        put(&mut m, "banana", b"2");

        let items = m.entries();
        assert_eq!(
            items,
            vec![
                Entry {
                    key: b"apple".to_vec(),
                    value: Some(b"1".to_vec()),
                },
                Entry {
                    key: b"banana".to_vec(),
                    value: Some(b"2".to_vec()),
                },
                Entry {
                    key: b"cherry".to_vec(),
                    value: Some(b"3".to_vec()),
                },
            ]
        );
    }

    #[test]
    fn capacity_tracking() {
        let mut m = MemTable::with_capacity(3);
        assert!(!m.at_capacity());
        put(&mut m, "a", b"1");
        put(&mut m, "b", b"2");
        put(&mut m, "c", b"3");
        assert!(m.at_capacity());
    }

    #[test]
    fn clear_resets() {
        let mut m = MemTable::new();
        put(&mut m, "apple", b"1");
        m.clear();
        assert_eq!(m.read("apple".as_bytes()), None);
        assert!(!m.at_capacity());
    }

    #[test]
    fn sstable_roundtrip() {
        let mut m = MemTable::new();
        put(&mut m, "apple", b"1");
        put(&mut m, "banana", b"2");
        delete(&mut m, "cherry");

        let sst = SSTable::from(&m);
        let bytes = sst.into_bytes().unwrap();
        let restored = SSTable::from_bytes(&bytes).unwrap();

        assert_eq!(restored.entries.len(), 3);
        assert_eq!(restored.entries[0].key, b"apple");
        assert_eq!(restored.entries[0].value, Some(b"1".to_vec()));
        assert_eq!(restored.entries[2].key, b"cherry");
        assert_eq!(restored.entries[2].value, None);
    }

    #[test]
    fn sstable_get() {
        let mut m = MemTable::new();
        put(&mut m, "apple", b"1");
        put(&mut m, "banana", b"2");
        put(&mut m, "cherry", b"3");

        let sst = SSTable::from(&m);
        assert_eq!(
            sst.get(b"banana"),
            Some(&Entry {
                key: b"banana".to_vec(),
                value: Some(b"2".to_vec()),
            })
        );
        assert_eq!(sst.get(b"missing"), None);
    }

    #[test]
    fn sstable_get_tombstone() {
        let mut m = MemTable::new();
        delete(&mut m, "deleted_key");

        let sst = SSTable::from(&m);
        let entry = sst.get(b"deleted_key").unwrap();
        assert_eq!(entry.value, None);
    }

    #[test]
    fn sstable_scan() {
        let mut m = MemTable::new();
        put(&mut m, "user/1", b"alice");
        put(&mut m, "user/2", b"bob");
        put(&mut m, "user/3", b"carol");
        put(&mut m, "order/1", b"o1");

        let sst = SSTable::from(&m);
        let results: Vec<_> = sst.scan(b"user/");
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].key, b"user/1");
        assert_eq!(results[1].key, b"user/2");
        assert_eq!(results[2].key, b"user/3");

        let empty = sst.scan(b"zzz");
        assert!(empty.is_empty());
    }
}
