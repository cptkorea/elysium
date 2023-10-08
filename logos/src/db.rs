use std::collections::BTreeMap;

use crate::Error;
use serde::{Deserialize, Serialize};

const CAPACITY: usize = 10_000;

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

pub struct MemTable {
    items: BTreeMap<String, u32>,
    size: usize,
}

impl MemTable {
    pub fn new() -> Self {
        Self {
            items: BTreeMap::new(),
            size: 0,
        }
    }

    pub fn write(&mut self, key: String, value: u32) -> Result<(), Error> {
        if self.size == CAPACITY {
            return Err(Error::MemTableFull);
        }

        self.items.insert(key, value);
        self.size += 1;
        Ok(())
    }

    pub fn read<S: AsRef<str>>(&self, key: S) -> Option<&u32> {
        self.items.get(key.as_ref())
    }

    pub fn items(&self) -> Vec<Entry> {
        self.items
            .iter()
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

impl From<&MemTable> for SSTable {
    fn from(value: &MemTable) -> Self {
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

    fn write(m: &mut MemTable, key: &str, value: u32) {
        m.write(String::from(key), value).unwrap();
    }

    #[test]
    fn simple_read_write() {
        let mut m = MemTable::new();
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
        let mut m = MemTable::new();
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
