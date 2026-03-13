use std::borrow::Borrow;
use std::collections::BTreeMap;

use crate::Error;
pub use crate::sorted_store::SortedStore;
use serde::{Deserialize, Serialize};

const DEFAULT_CAPACITY: usize = 10_000;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Entry {
    pub key: String,
    pub value: u32,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.key.eq(&other.key)
    }
}

pub struct MemTable<K: Ord, V, S: SortedStore<K, V> = BTreeMap<K, V>> {
    items: S,
    size: usize,
    capacity: usize,
    _key: std::marker::PhantomData<K>,
    _value: std::marker::PhantomData<V>,
}

impl<K: Ord, V> MemTable<K, V, BTreeMap<K, V>> {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            items: BTreeMap::new(),
            size: 0,
            capacity,
            _key: std::marker::PhantomData,
            _value: std::marker::PhantomData,
        }
    }
}

impl<K: Ord, V, S: SortedStore<K, V>> MemTable<K, V, S> {
    pub fn with_store(capacity: usize, store: S) -> Self {
        Self {
            items: store,
            size: 0,
            capacity,
            _key: std::marker::PhantomData,
            _value: std::marker::PhantomData,
        }
    }

    pub fn write(&mut self, key: K, value: V) {
        self.items.insert(key, value);
        self.size += 1;
    }

    pub fn read<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.items.get(key)
    }

    pub fn at_capacity(&self) -> bool {
        self.size >= self.capacity
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.size = 0;
    }
}

impl<S: SortedStore<String, u32>> MemTable<String, u32, S> {
    pub fn items(&self) -> Vec<Entry> {
        self.items
            .iter_sorted()
            .map(|(k, v)| Entry {
                key: k.to_owned(),
                value: v.to_owned(),
            })
            .collect()
    }
}

#[derive(Deserialize, Serialize)]
pub struct SSTable {
    entries: Vec<Entry>,
}

impl<S: SortedStore<String, u32>> From<&MemTable<String, u32, S>> for SSTable {
    fn from(value: &MemTable<String, u32, S>) -> Self {
        SSTable {
            entries: value.items(),
        }
    }
}

impl SSTable {
    pub fn into_bytes(&self) -> Result<Vec<u8>, Error> {
        bincode::serialize(self).map_err(|_| Error::BincodeError)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn write<S: SortedStore<String, u32>>(m: &mut MemTable<String, u32, S>, key: &str, value: u32) {
        m.write(String::from(key), value);
    }

    #[test]
    fn simple_read_write() {
        let mut m: MemTable<String, u32> = MemTable::new();
        write(&mut m, "apple", 1);
        write(&mut m, "banana", 2);
        write(&mut m, "cactus", 3);

        assert_eq!(Some(&1), m.read("apple"));
        assert_eq!(Some(&2), m.read("banana"));
        assert_eq!(Some(&3), m.read("cactus"));
        assert_eq!(None, m.read("dummy"));

        write(&mut m, "apple", 5);
        assert_eq!(Some(&5), m.read("apple"));
        assert_eq!(Some(&2), m.read("banana"));
        assert_eq!(Some(&3), m.read("cactus"));
        assert_eq!(None, m.read("dummy"));
    }

    #[test]
    fn items() {
        let mut m: MemTable<String, u32> = MemTable::new();
        write(&mut m, "apple", 1);
        write(&mut m, "banana", 2);
        write(&mut m, "cactus", 3);
        write(&mut m, "apple", 5);

        let items = m.items();
        assert_eq!(
            items,
            vec![
                Entry {
                    key: String::from("apple"),
                    value: 5
                },
                Entry {
                    key: String::from("banana"),
                    value: 2
                },
                Entry {
                    key: String::from("cactus"),
                    value: 3
                },
            ]
        )
    }
}
