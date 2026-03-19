#[cfg(test)]
mod memtable_conformance {
    use ontos::db::{Entry, MemTable, SortedStore};

    fn write_then_read<S: SortedStore<Vec<u8>, Option<Vec<u8>>>>(mut m: MemTable<S>) {
        m.write(b"apple".to_vec(), Some(b"1".to_vec()));
        m.write(b"banana".to_vec(), Some(b"2".to_vec()));
        m.write(b"cherry".to_vec(), Some(b"3".to_vec()));

        assert_eq!(m.read(b"apple".as_slice()), Some(&Some(b"1".to_vec())));
        assert_eq!(m.read(b"banana".as_slice()), Some(&Some(b"2".to_vec())));
        assert_eq!(m.read(b"cherry".as_slice()), Some(&Some(b"3".to_vec())));
    }

    fn overwrite<S: SortedStore<Vec<u8>, Option<Vec<u8>>>>(mut m: MemTable<S>) {
        m.write(b"apple".to_vec(), Some(b"1".to_vec()));
        m.write(b"apple".to_vec(), Some(b"5".to_vec()));

        assert_eq!(m.read(b"apple".as_slice()), Some(&Some(b"5".to_vec())));
    }

    fn read_miss<S: SortedStore<Vec<u8>, Option<Vec<u8>>>>(m: MemTable<S>) {
        assert_eq!(m.read(b"ghost".as_slice()), None);
    }

    fn items_sorted<S: SortedStore<Vec<u8>, Option<Vec<u8>>>>(mut m: MemTable<S>) {
        m.write(b"cherry".to_vec(), Some(b"3".to_vec()));
        m.write(b"apple".to_vec(), Some(b"1".to_vec()));
        m.write(b"banana".to_vec(), Some(b"2".to_vec()));

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

    fn capacity_tracking<S: SortedStore<Vec<u8>, Option<Vec<u8>>>>(mut m: MemTable<S>) {
        assert!(!m.at_capacity());
        for i in 0..5u8 {
            m.write(vec![i], Some(vec![i]));
        }
        assert!(m.at_capacity());
    }

    fn clear_resets<S: SortedStore<Vec<u8>, Option<Vec<u8>>>>(mut m: MemTable<S>) {
        m.write(b"apple".to_vec(), Some(b"1".to_vec()));
        m.write(b"banana".to_vec(), Some(b"2".to_vec()));
        m.clear();

        assert_eq!(m.read(b"apple".as_slice()), None);
        assert_eq!(m.read(b"banana".as_slice()), None);
        assert!(!m.at_capacity());
    }

    fn tombstone<S: SortedStore<Vec<u8>, Option<Vec<u8>>>>(mut m: MemTable<S>) {
        m.write(b"apple".to_vec(), Some(b"1".to_vec()));
        m.write(b"apple".to_vec(), None);

        assert_eq!(m.read(b"apple".as_slice()), Some(&None));
    }

    mod btreemap {
        use super::*;

        fn make() -> MemTable {
            MemTable::with_capacity(5)
        }

        #[test]
        fn test_write_then_read() {
            write_then_read(make());
        }
        #[test]
        fn test_overwrite() {
            overwrite(make());
        }
        #[test]
        fn test_read_miss() {
            read_miss(make());
        }
        #[test]
        fn test_items_sorted() {
            items_sorted(make());
        }
        #[test]
        fn test_capacity_tracking() {
            capacity_tracking(make());
        }
        #[test]
        fn test_clear_resets() {
            clear_resets(make());
        }
        #[test]
        fn test_tombstone() {
            tombstone(make());
        }
    }

    mod skiplist {
        use super::*;
        use elysium_common::skiplist::SkipList;

        fn make() -> MemTable<SkipList<Vec<u8>, Option<Vec<u8>>>> {
            MemTable::with_store(5, SkipList::new())
        }

        #[test]
        fn test_write_then_read() {
            write_then_read(make());
        }
        #[test]
        fn test_overwrite() {
            overwrite(make());
        }
        #[test]
        fn test_read_miss() {
            read_miss(make());
        }
        #[test]
        fn test_items_sorted() {
            items_sorted(make());
        }
        #[test]
        fn test_capacity_tracking() {
            capacity_tracking(make());
        }
        #[test]
        fn test_clear_resets() {
            clear_resets(make());
        }
        #[test]
        fn test_tombstone() {
            tombstone(make());
        }
    }
}
