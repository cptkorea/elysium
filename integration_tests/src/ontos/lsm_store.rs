#[cfg(test)]
mod memtable_conformance {
    use ontos::db::{Entry, MemTable, SortedStore};

    fn write_then_read<S: SortedStore<String, u32>>(mut m: MemTable<String, u32, S>) {
        m.write("apple".into(), 1);
        m.write("banana".into(), 2);
        m.write("cherry".into(), 3);

        assert_eq!(m.read("apple"), Some(&1));
        assert_eq!(m.read("banana"), Some(&2));
        assert_eq!(m.read("cherry"), Some(&3));
    }

    fn overwrite<S: SortedStore<String, u32>>(mut m: MemTable<String, u32, S>) {
        m.write("apple".into(), 1);
        m.write("apple".into(), 5);

        assert_eq!(m.read("apple"), Some(&5));
    }

    fn read_miss<S: SortedStore<String, u32>>(m: MemTable<String, u32, S>) {
        assert_eq!(m.read("ghost"), None);
    }

    fn items_sorted<S: SortedStore<String, u32>>(mut m: MemTable<String, u32, S>) {
        m.write("cherry".into(), 3);
        m.write("apple".into(), 1);
        m.write("banana".into(), 2);

        let items = m.items();
        assert_eq!(
            items,
            vec![
                Entry { key: "apple".into(), value: 1 },
                Entry { key: "banana".into(), value: 2 },
                Entry { key: "cherry".into(), value: 3 },
            ]
        );
    }

    fn capacity_tracking<S: SortedStore<String, u32>>(mut m: MemTable<String, u32, S>) {
        assert!(!m.at_capacity());
        for i in 0..5 {
            m.write(i.to_string(), i);
        }
        assert!(m.at_capacity());
    }

    fn clear_resets<S: SortedStore<String, u32>>(mut m: MemTable<String, u32, S>) {
        m.write("apple".into(), 1);
        m.write("banana".into(), 2);
        m.clear();

        assert_eq!(m.read("apple"), None);
        assert_eq!(m.read("banana"), None);
        assert!(!m.at_capacity());
    }

    mod btreemap {
        use super::*;

        fn make() -> MemTable<String, u32> {
            MemTable::with_capacity(5)
        }

        #[test]
        fn test_write_then_read() { write_then_read(make()); }
        #[test]
        fn test_overwrite() { overwrite(make()); }
        #[test]
        fn test_read_miss() { read_miss(make()); }
        #[test]
        fn test_items_sorted() { items_sorted(make()); }
        #[test]
        fn test_capacity_tracking() { capacity_tracking(make()); }
        #[test]
        fn test_clear_resets() { clear_resets(make()); }
    }

    mod skiplist {
        use super::*;
        use elysium_common::skiplist::SkipList;

        fn make() -> MemTable<String, u32, SkipList<String, u32>> {
            MemTable::with_store(5, SkipList::new())
        }

        #[test]
        fn test_write_then_read() { write_then_read(make()); }
        #[test]
        fn test_overwrite() { overwrite(make()); }
        #[test]
        fn test_read_miss() { read_miss(make()); }
        #[test]
        fn test_items_sorted() { items_sorted(make()); }
        #[test]
        fn test_capacity_tracking() { capacity_tracking(make()); }
        #[test]
        fn test_clear_resets() { clear_resets(make()); }
    }
}
