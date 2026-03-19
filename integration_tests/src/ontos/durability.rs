#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ontos::driver::Driver;
    use ontos::DurabilityMode;
    use tempfile::TempDir;

    #[test]
    fn sync_put_flush_reopen_get() {
        let dir = TempDir::new().unwrap();

        {
            let mut d = Driver::open(dir.path(), DurabilityMode::Sync).unwrap();
            d.put(b"a".to_vec(), b"1".to_vec()).unwrap();
            d.put(b"b".to_vec(), b"2".to_vec()).unwrap();
            d.put(b"c".to_vec(), b"3".to_vec()).unwrap();
            d.flush_table().unwrap();
        }

        let d = Driver::open(dir.path(), DurabilityMode::Sync).unwrap();
        assert_eq!(d.get(b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(d.get(b"b").unwrap(), Some(b"2".to_vec()));
        assert_eq!(d.get(b"c").unwrap(), Some(b"3".to_vec()));
        assert_eq!(d.get(b"missing").unwrap(), None);
    }

    #[test]
    fn sync_wal_recovery() {
        let dir = TempDir::new().unwrap();

        {
            let mut d = Driver::open(dir.path(), DurabilityMode::Sync).unwrap();
            d.put(b"x".to_vec(), b"10".to_vec()).unwrap();
            d.put(b"y".to_vec(), b"20".to_vec()).unwrap();
            d.put(b"z".to_vec(), b"30".to_vec()).unwrap();
            // No flush — data only lives in the WAL.
        }

        let d = Driver::open(dir.path(), DurabilityMode::Sync).unwrap();
        assert_eq!(d.get(b"x").unwrap(), Some(b"10".to_vec()));
        assert_eq!(d.get(b"y").unwrap(), Some(b"20".to_vec()));
        assert_eq!(d.get(b"z").unwrap(), Some(b"30".to_vec()));
    }

    #[test]
    fn sync_delete_shadows_across_reopen() {
        let dir = TempDir::new().unwrap();

        {
            let mut d = Driver::open(dir.path(), DurabilityMode::Sync).unwrap();
            d.put(b"key".to_vec(), b"alive".to_vec()).unwrap();
            d.flush_table().unwrap();
            d.delete(b"key".to_vec()).unwrap();
            // Tombstone is in the WAL, live value is in the SSTable.
        }

        let d = Driver::open(dir.path(), DurabilityMode::Sync).unwrap();
        assert_eq!(d.get(b"key").unwrap(), None);
    }

    #[test]
    fn async_put_flush_reopen_get() {
        let dir = TempDir::new().unwrap();
        let mode = DurabilityMode::Async(Duration::from_millis(50));

        {
            let mut d = Driver::open(dir.path(), mode.clone()).unwrap();
            d.put(b"a".to_vec(), b"1".to_vec()).unwrap();
            d.put(b"b".to_vec(), b"2".to_vec()).unwrap();
            d.flush_table().unwrap();
        }

        let d = Driver::open(dir.path(), mode).unwrap();
        assert_eq!(d.get(b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(d.get(b"b").unwrap(), Some(b"2".to_vec()));
    }

    #[test]
    fn async_wal_recovery() {
        let dir = TempDir::new().unwrap();
        let mode = DurabilityMode::Async(Duration::from_millis(50));

        {
            let mut d = Driver::open(dir.path(), mode.clone()).unwrap();
            d.put(b"p".to_vec(), b"100".to_vec()).unwrap();
            d.put(b"q".to_vec(), b"200".to_vec()).unwrap();

            // Wait for the background flusher to sync the WAL to disk.
            std::thread::sleep(Duration::from_millis(100));
        }

        let d = Driver::open(dir.path(), mode).unwrap();
        assert_eq!(d.get(b"p").unwrap(), Some(b"100".to_vec()));
        assert_eq!(d.get(b"q").unwrap(), Some(b"200".to_vec()));
    }

    #[test]
    fn volatile_put_flush_reopen_get() {
        let dir = TempDir::new().unwrap();

        {
            let mut d = Driver::open(dir.path(), DurabilityMode::Volatile).unwrap();
            d.put(b"a".to_vec(), b"1".to_vec()).unwrap();
            d.put(b"b".to_vec(), b"2".to_vec()).unwrap();
            d.flush_table().unwrap();
        }

        let d = Driver::open(dir.path(), DurabilityMode::Volatile).unwrap();
        assert_eq!(d.get(b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(d.get(b"b").unwrap(), Some(b"2".to_vec()));
    }

    #[test]
    fn mixed_workload_across_flushes() {
        let dir = TempDir::new().unwrap();

        {
            let mut d = Driver::open(dir.path(), DurabilityMode::Sync).unwrap();

            // Batch 1: three keys, flushed to SSTable.
            d.put(b"user/1".to_vec(), b"alice".to_vec()).unwrap();
            d.put(b"user/2".to_vec(), b"bob".to_vec()).unwrap();
            d.put(b"user/3".to_vec(), b"carol".to_vec()).unwrap();
            d.flush_table().unwrap();

            // Batch 2: overwrite one, delete another, add a new one.
            d.put(b"user/2".to_vec(), b"bob-v2".to_vec()).unwrap();
            d.delete(b"user/3".to_vec()).unwrap();
            d.put(b"user/4".to_vec(), b"dave".to_vec()).unwrap();
            d.flush_table().unwrap();

            // Batch 3: delete the overwritten key, only in WAL.
            d.delete(b"user/2".to_vec()).unwrap();
        }

        let d = Driver::open(dir.path(), DurabilityMode::Sync).unwrap();
        assert_eq!(d.get(b"user/1").unwrap(), Some(b"alice".to_vec()));
        assert_eq!(d.get(b"user/2").unwrap(), None);
        assert_eq!(d.get(b"user/3").unwrap(), None);
        assert_eq!(d.get(b"user/4").unwrap(), Some(b"dave".to_vec()));
    }

    #[test]
    fn scan_after_reopen() {
        let dir = TempDir::new().unwrap();

        {
            let mut d = Driver::open(dir.path(), DurabilityMode::Sync).unwrap();

            // Flushed to SSTable.
            d.put(b"order/1".to_vec(), b"o1".to_vec()).unwrap();
            d.put(b"order/2".to_vec(), b"o2".to_vec()).unwrap();
            d.put(b"item/1".to_vec(), b"widget".to_vec()).unwrap();
            d.flush_table().unwrap();

            // Left in WAL only.
            d.put(b"order/3".to_vec(), b"o3".to_vec()).unwrap();
            d.put(b"order/2".to_vec(), b"o2-updated".to_vec()).unwrap();
        }

        let d = Driver::open(dir.path(), DurabilityMode::Sync).unwrap();

        let orders = d.scan(b"order/").unwrap();
        assert_eq!(orders.len(), 3);
        assert_eq!(orders[0], (b"order/1".to_vec(), b"o1".to_vec()));
        assert_eq!(orders[1], (b"order/2".to_vec(), b"o2-updated".to_vec()));
        assert_eq!(orders[2], (b"order/3".to_vec(), b"o3".to_vec()));

        let items = d.scan(b"item/").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0], (b"item/1".to_vec(), b"widget".to_vec()));
    }
}
