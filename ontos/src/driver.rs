use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::db::{MemTable, SSTable, SortedStore};
use crate::wal::{Wal, WalRecord};
use crate::Error;

/// Top-level LSM-tree engine that coordinates writes through a [`MemTable`],
/// persists mutations via a [`Wal`], and flushes full MemTables to
/// [`SSTable`] files on disk.
///
/// ## Write Path
///
/// 1. The mutation is appended to the WAL (fsynced for durability).
/// 2. The mutation is applied to the in-memory MemTable.
/// 3. When the MemTable reaches capacity, it is flushed to a `.sst` file
///    and the WAL is rotated (truncated).
///
/// ## Read Path
///
/// 1. The MemTable is checked first (newest data).
/// 2. SSTables are checked from newest to oldest.
/// 3. The first match wins — newer entries shadow older ones.
/// 4. Tombstones (`value: None`) are respected: a tombstone in a newer
///    source means the key is deleted, even if an older SSTable has a
///    live value.
///
/// ## Recovery
///
/// On startup via [`Driver::open`], the WAL is replayed to reconstruct
/// the MemTable to its pre-crash state. Existing SSTable files on disk
/// are discovered and tracked for reads.
///
/// ## Data Directory Layout
///
/// ```text
/// {data_dir}/
///   wal.log          — active write-ahead log
///   000000.sst       — SSTable files, numbered sequentially
///   000001.sst
///   ...
/// ```
pub struct Driver<S: SortedStore<Vec<u8>, Option<Vec<u8>>> = BTreeMap<Vec<u8>, Option<Vec<u8>>>> {
    master: MemTable<S>,
    wal: Wal,
    data_dir: PathBuf,
    /// Paths to SSTable files, ordered from oldest (index 0) to newest.
    sst_paths: Vec<PathBuf>,
    offset: usize,
}

impl Driver<BTreeMap<Vec<u8>, Option<Vec<u8>>>> {
    /// Opens or creates a `Driver` rooted at `data_dir`.
    ///
    /// If the directory already exists and contains a WAL file, the WAL
    /// is replayed to reconstruct the MemTable. Existing `.sst` files
    /// are discovered so the offset counter continues from where it left
    /// off.
    ///
    /// If the directory does not exist, it is created.
    pub fn open(data_dir: impl Into<PathBuf>) -> Result<Self, Error> {
        let data_dir = data_dir.into();
        fs::create_dir_all(&data_dir)?;

        let wal_path = data_dir.join("wal.log");
        let records = Wal::recover(&wal_path)?;

        let mut memtable = MemTable::new();
        for record in records {
            match record {
                WalRecord::Put { key, value } => memtable.write(key, Some(value)),
                WalRecord::Delete { key } => memtable.write(key, None),
            }
        }

        let wal = Wal::open(&wal_path)?;
        let sst_paths = discover_sst_files(&data_dir);
        let offset = sst_paths.len();

        Ok(Self {
            master: memtable,
            wal,
            data_dir,
            sst_paths,
            offset,
        })
    }
}

impl<S: SortedStore<Vec<u8>, Option<Vec<u8>>>> Driver<S> {
    /// Opens a `Driver` with a caller-supplied [`MemTable`] implementation.
    ///
    /// Unlike [`Driver::open`], this constructor does **not** replay the
    /// WAL into the provided MemTable — the caller is responsible for any
    /// pre-population. This is primarily useful for testing with
    /// alternative `SortedStore` backends.
    pub fn open_with_memtable(
        data_dir: impl Into<PathBuf>,
        memtable: MemTable<S>,
    ) -> Result<Self, Error> {
        let data_dir = data_dir.into();
        fs::create_dir_all(&data_dir)?;

        let wal_path = data_dir.join("wal.log");
        let wal = Wal::open(&wal_path)?;
        let sst_paths = discover_sst_files(&data_dir);
        let offset = sst_paths.len();

        Ok(Self {
            master: memtable,
            wal,
            data_dir,
            sst_paths,
            offset,
        })
    }

    /// Writes a key-value pair to the store.
    ///
    /// The mutation is first appended to the WAL for durability, then
    /// applied to the in-memory MemTable. If the MemTable is at capacity
    /// before this write, it is flushed to an SSTable first.
    ///
    /// To remove a key, use [`delete`](Self::delete) instead.
    pub fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), Error> {
        if self.master.at_capacity() {
            self.flush_table()?;
        }

        self.wal.append(&WalRecord::Put {
            key: key.clone(),
            value: value.clone(),
        })?;
        self.master.write(key, Some(value));
        Ok(())
    }

    /// Marks a key as deleted by writing a tombstone.
    ///
    /// The tombstone is first appended to the WAL, then applied to the
    /// MemTable. During reads, the tombstone will shadow any older live
    /// entry for the same key.
    pub fn delete(&mut self, key: Vec<u8>) -> Result<(), Error> {
        if self.master.at_capacity() {
            self.flush_table()?;
        }

        self.wal.append(&WalRecord::Delete { key: key.clone() })?;
        self.master.write(key, None);
        Ok(())
    }

    /// Flushes the current MemTable to an SSTable file and rotates the WAL.
    ///
    /// The SSTable is written synchronously to `{data_dir}/{offset}.sst`.
    /// After the SSTable is durable on disk, the WAL is truncated since
    /// those records are now redundant.
    pub fn flush_table(&mut self) -> Result<(), Error> {
        let sst = SSTable::from(&self.master);
        self.master.clear();

        let bytes = sst.into_bytes()?;
        let sst_path = self.data_dir.join(format!("{:06}.sst", self.offset));
        self.offset += 1;

        write_sst(&sst_path, &bytes)?;
        self.sst_paths.push(sst_path);
        self.wal.rotate()?;

        Ok(())
    }

    /// Retrieves the value for a single key, or `None` if the key is
    /// absent or has been deleted.
    ///
    /// The lookup order is:
    /// 1. Active MemTable (newest data).
    /// 2. SSTables from newest to oldest.
    ///
    /// The first source that contains the key wins. If that source holds
    /// a tombstone (`value: None`), the key is considered deleted and
    /// `None` is returned — older SSTables are not consulted.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        if let Some(value) = self.master.read(key) {
            return Ok(value.clone());
        }

        for sst_path in self.sst_paths.iter().rev() {
            let sst = SSTable::from_file(sst_path)?;
            if let Some(entry) = sst.get(key) {
                return Ok(entry.value.clone());
            }
        }

        Ok(None)
    }

    /// Returns all live key-value pairs whose key starts with `prefix`.
    ///
    /// Merges results across the MemTable and all SSTables. When a key
    /// appears in multiple sources, the newest entry wins. Tombstones
    /// are respected: a deleted key is excluded from the result even if
    /// an older SSTable has a live value for it.
    ///
    /// Results are returned in sorted key order.
    pub fn scan(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Error> {
        let mut merged: HashMap<Vec<u8>, Option<Vec<u8>>> = HashMap::new();

        // SSTables oldest-to-newest so newer entries overwrite older ones.
        for sst_path in &self.sst_paths {
            let sst = SSTable::from_file(sst_path)?;
            for entry in sst.scan(prefix) {
                merged.insert(entry.key.clone(), entry.value.clone());
            }
        }

        // MemTable is newest — overwrites everything.
        for entry in self.master.entries() {
            if entry.key.starts_with(prefix) {
                merged.insert(entry.key, entry.value);
            }
        }

        let mut results: Vec<(Vec<u8>, Vec<u8>)> = merged
            .into_iter()
            .filter_map(|(k, v)| v.map(|val| (k, val)))
            .collect();
        results.sort_by(|(a, _), (b, _)| a.cmp(b));

        Ok(results)
    }
}

/// Writes serialized SSTable bytes to a file, fsyncing for durability.
fn write_sst(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_data()?;
    Ok(())
}

/// Discovers all `.sst` files in a directory, returning their paths
/// sorted by filename (oldest to newest).
fn discover_sst_files(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map_or(false, |ext| ext == "sst"))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    paths
}

#[cfg(test)]
mod test {
    use crate::db::Entry;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn put_and_flush() {
        let dir = TempDir::new().unwrap();
        let mut driver = Driver::open(dir.path()).unwrap();

        driver.put(b"a".to_vec(), b"1".to_vec()).unwrap();
        driver.put(b"b".to_vec(), b"2".to_vec()).unwrap();

        assert_eq!(
            driver.master.entries(),
            vec![
                Entry {
                    key: b"a".to_vec(),
                    value: Some(b"1".to_vec()),
                },
                Entry {
                    key: b"b".to_vec(),
                    value: Some(b"2".to_vec()),
                },
            ]
        );

        driver.flush_table().unwrap();
        assert!(driver.master.entries().is_empty());
        assert!(dir.path().join("000000.sst").exists());
    }

    #[test]
    fn auto_flush_on_capacity() {
        let dir = TempDir::new().unwrap();
        let mut driver = Driver {
            master: MemTable::with_capacity(2),
            wal: Wal::open(dir.path().join("wal.log")).unwrap(),
            data_dir: dir.path().to_path_buf(),
            sst_paths: Vec::new(),
            offset: 0,
        };

        driver.put(b"a".to_vec(), b"1".to_vec()).unwrap();
        driver.put(b"b".to_vec(), b"2".to_vec()).unwrap();
        assert!(driver.master.at_capacity());

        // Third write triggers flush, then inserts into the fresh MemTable.
        driver.put(b"c".to_vec(), b"3".to_vec()).unwrap();
        assert_eq!(
            driver.master.entries(),
            vec![Entry {
                key: b"c".to_vec(),
                value: Some(b"3".to_vec()),
            }]
        );
        assert!(dir.path().join("000000.sst").exists());
    }

    #[test]
    fn delete_writes_tombstone() {
        let dir = TempDir::new().unwrap();
        let mut driver = Driver::open(dir.path()).unwrap();

        driver.put(b"key".to_vec(), b"val".to_vec()).unwrap();
        driver.delete(b"key".to_vec()).unwrap();

        let entries = driver.master.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0],
            Entry {
                key: b"key".to_vec(),
                value: None,
            }
        );
    }

    #[test]
    fn wal_recovery() {
        let dir = TempDir::new().unwrap();

        {
            let mut driver = Driver::open(dir.path()).unwrap();
            driver.put(b"x".to_vec(), b"10".to_vec()).unwrap();
            driver.put(b"y".to_vec(), b"20".to_vec()).unwrap();
            driver.delete(b"x".to_vec()).unwrap();
        }

        let driver = Driver::open(dir.path()).unwrap();
        assert_eq!(driver.master.read(b"x".as_slice()), Some(&None));
        assert_eq!(
            driver.master.read(b"y".as_slice()),
            Some(&Some(b"20".to_vec())),
        );
    }

    #[test]
    fn wal_rotated_after_flush() {
        let dir = TempDir::new().unwrap();

        {
            let mut driver = Driver::open(dir.path()).unwrap();
            driver.put(b"a".to_vec(), b"1".to_vec()).unwrap();
            driver.flush_table().unwrap();
        }

        let driver = Driver::open(dir.path()).unwrap();
        assert!(driver.master.entries().is_empty());
    }

    #[test]
    fn offset_resumes_after_reopen() {
        let dir = TempDir::new().unwrap();

        {
            let mut driver = Driver::open(dir.path()).unwrap();
            driver.put(b"a".to_vec(), b"1".to_vec()).unwrap();
            driver.flush_table().unwrap();
            assert!(dir.path().join("000000.sst").exists());
        }

        {
            let mut driver = Driver::open(dir.path()).unwrap();
            driver.put(b"b".to_vec(), b"2".to_vec()).unwrap();
            driver.flush_table().unwrap();
            assert!(dir.path().join("000001.sst").exists());
        }
    }

    #[test]
    fn get_from_memtable() {
        let dir = TempDir::new().unwrap();
        let mut driver = Driver::open(dir.path()).unwrap();

        driver.put(b"key".to_vec(), b"val".to_vec()).unwrap();
        assert_eq!(driver.get(b"key").unwrap(), Some(b"val".to_vec()));
        assert_eq!(driver.get(b"missing").unwrap(), None);
    }

    #[test]
    fn get_from_sstable() {
        let dir = TempDir::new().unwrap();
        let mut driver = Driver::open(dir.path()).unwrap();

        driver.put(b"a".to_vec(), b"1".to_vec()).unwrap();
        driver.put(b"b".to_vec(), b"2".to_vec()).unwrap();
        driver.flush_table().unwrap();

        assert_eq!(driver.get(b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(driver.get(b"b").unwrap(), Some(b"2".to_vec()));
        assert_eq!(driver.get(b"c").unwrap(), None);
    }

    #[test]
    fn get_memtable_shadows_sstable() {
        let dir = TempDir::new().unwrap();
        let mut driver = Driver::open(dir.path()).unwrap();

        driver.put(b"k".to_vec(), b"old".to_vec()).unwrap();
        driver.flush_table().unwrap();

        driver.put(b"k".to_vec(), b"new".to_vec()).unwrap();
        assert_eq!(driver.get(b"k").unwrap(), Some(b"new".to_vec()));
    }

    #[test]
    fn get_newer_sstable_shadows_older() {
        let dir = TempDir::new().unwrap();
        let mut driver = Driver::open(dir.path()).unwrap();

        driver.put(b"k".to_vec(), b"v1".to_vec()).unwrap();
        driver.flush_table().unwrap();

        driver.put(b"k".to_vec(), b"v2".to_vec()).unwrap();
        driver.flush_table().unwrap();

        assert_eq!(driver.get(b"k").unwrap(), Some(b"v2".to_vec()));
    }

    #[test]
    fn get_respects_tombstone() {
        let dir = TempDir::new().unwrap();
        let mut driver = Driver::open(dir.path()).unwrap();

        driver.put(b"k".to_vec(), b"alive".to_vec()).unwrap();
        driver.flush_table().unwrap();

        driver.delete(b"k".to_vec()).unwrap();
        assert_eq!(driver.get(b"k").unwrap(), None);
    }

    #[test]
    fn get_respects_tombstone_in_sstable() {
        let dir = TempDir::new().unwrap();
        let mut driver = Driver::open(dir.path()).unwrap();

        driver.put(b"k".to_vec(), b"alive".to_vec()).unwrap();
        driver.flush_table().unwrap();

        driver.delete(b"k".to_vec()).unwrap();
        driver.flush_table().unwrap();

        assert_eq!(driver.get(b"k").unwrap(), None);
    }

    #[test]
    fn scan_merges_memtable_and_sstables() {
        let dir = TempDir::new().unwrap();
        let mut driver = Driver::open(dir.path()).unwrap();

        driver.put(b"user/1".to_vec(), b"alice".to_vec()).unwrap();
        driver.put(b"user/2".to_vec(), b"bob".to_vec()).unwrap();
        driver.flush_table().unwrap();

        driver.put(b"user/3".to_vec(), b"carol".to_vec()).unwrap();
        driver.put(b"order/1".to_vec(), b"o1".to_vec()).unwrap();

        let results = driver.scan(b"user/").unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (b"user/1".to_vec(), b"alice".to_vec()));
        assert_eq!(results[1], (b"user/2".to_vec(), b"bob".to_vec()));
        assert_eq!(results[2], (b"user/3".to_vec(), b"carol".to_vec()));
    }

    #[test]
    fn scan_respects_tombstones() {
        let dir = TempDir::new().unwrap();
        let mut driver = Driver::open(dir.path()).unwrap();

        driver.put(b"user/1".to_vec(), b"alice".to_vec()).unwrap();
        driver.put(b"user/2".to_vec(), b"bob".to_vec()).unwrap();
        driver.flush_table().unwrap();

        driver.delete(b"user/1".to_vec()).unwrap();

        let results = driver.scan(b"user/").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], (b"user/2".to_vec(), b"bob".to_vec()));
    }

    #[test]
    fn scan_newer_overwrites_older() {
        let dir = TempDir::new().unwrap();
        let mut driver = Driver::open(dir.path()).unwrap();

        driver.put(b"k".to_vec(), b"old".to_vec()).unwrap();
        driver.flush_table().unwrap();

        driver.put(b"k".to_vec(), b"new".to_vec()).unwrap();

        let results = driver.scan(b"k").unwrap();
        assert_eq!(results, vec![(b"k".to_vec(), b"new".to_vec())]);
    }

    #[test]
    fn get_after_reopen() {
        let dir = TempDir::new().unwrap();

        {
            let mut driver = Driver::open(dir.path()).unwrap();
            driver.put(b"a".to_vec(), b"1".to_vec()).unwrap();
            driver.flush_table().unwrap();
            driver.put(b"b".to_vec(), b"2".to_vec()).unwrap();
        }

        let driver = Driver::open(dir.path()).unwrap();
        assert_eq!(driver.get(b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(driver.get(b"b").unwrap(), Some(b"2".to_vec()));
    }
}
