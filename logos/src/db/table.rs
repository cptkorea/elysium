use super::KeyValue;

const TABLE_SIZE: usize = 1000;

pub struct MemTable {
    items: Vec<KeyValue>,
    size: usize,
}

impl MemTable {
    pub fn new() -> Self {
        Self {
            items: Vec::with_capacity(TABLE_SIZE),
            size: 0,
        }
    }

    pub fn write(&mut self, key: String, value: u32) {
        self.items.push(KeyValue { key, value });
        self.size += 1;
    }

    pub fn read<S: AsRef<str>>(&self, key: S) -> Option<u32> {
        for kv in self.items.iter().rev() {
            if &kv.key == key.as_ref() {
                return Some(kv.value);
            }
        }
        None
    }

    pub fn items(&self) -> Vec<KeyValue> {
        let mut items: Vec<KeyValue> = Vec::new();
        for kv in self.items.iter().rev() {
            if !items.iter().any(|s| s.key == kv.key) {
                items.push(kv.to_owned())
            }
        }
        items
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

        assert_eq!(Some(1), m.read("apple"));
        assert_eq!(Some(2), m.read("banana"));
        assert_eq!(Some(3), m.read("cactus"));
        assert_eq!(None, m.read("dummy"));

        m.write(String::from("apple"), 5);
        assert_eq!(Some(5), m.read("apple"));
        assert_eq!(Some(2), m.read("banana"));
        assert_eq!(Some(3), m.read("cactus"));
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
                KeyValue {
                    key: String::from("apple"),
                    value: 5
                },
                KeyValue {
                    key: String::from("cactus"),
                    value: 3
                },
                KeyValue {
                    key: String::from("banana"),
                    value: 2
                },
            ]
        )
    }
}
