use std::collections::BTreeMap;

#[derive(Debug, Clone)]
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

    pub fn write(&mut self, key: String, value: u32) {
        self.items.insert(key, value);
        self.size += 1;
    }

    pub fn read<S: AsRef<str>>(&self, key: S) -> Option<&u32> {
        self.items.get(key.as_ref())
    }

    #[cfg(test)]
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn simple_read_write() {
        let mut m = MemTable::new();
        m.write(String::from("apple"), 1);
        m.write(String::from("banana"), 2);
        m.write(String::from("cactus"), 3);

        assert_eq!(Some(&1), m.read("apple"));
        assert_eq!(Some(&2), m.read("banana"));
        assert_eq!(Some(&3), m.read("cactus"));
        assert_eq!(None, m.read("dummy"));

        m.write(String::from("apple"), 5);
        assert_eq!(Some(&5), m.read("apple"));
        assert_eq!(Some(&2), m.read("banana"));
        assert_eq!(Some(&3), m.read("cactus"));
        assert_eq!(None, m.read("dummy"));
    }

    #[test]
    fn items() {
        let mut m = MemTable::new();
        m.write(String::from("apple"), 1);
        m.write(String::from("banana"), 2);
        m.write(String::from("cactus"), 3);
        m.write(String::from("apple"), 5);

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
