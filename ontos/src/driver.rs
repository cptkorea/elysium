use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;

use crate::db::{MemTable, SSTable, SortedStore};
use crate::Error;

pub struct Driver<S: SortedStore<String, u32> = BTreeMap<String, u32>> {
    master: MemTable<String, u32, S>,
    offset: usize,
}

impl Driver<BTreeMap<String, u32>> {
    pub fn new() -> Self {
        Self {
            master: MemTable::new(),
            offset: 0,
        }
    }
}

impl<S: SortedStore<String, u32>> Driver<S> {
    pub fn with_memtable(memtable: MemTable<String, u32, S>) -> Self {
        Self {
            master: memtable,
            offset: 0,
        }
    }

    pub async fn write(&mut self, key: String, value: u32) -> Result<(), Error> {
        if self.master.at_capacity() {
            self.flush_table().await?;
        }
        self.master.write(key, value);
        Ok(())
    }

    pub async fn flush_table(&mut self) -> Result<(), Error> {
        let sst = SSTable::from(&self.master);
        self.master.clear();

        let bytes = sst.into_bytes()?;
        let offset = self.offset;
        self.offset += 1;

        tokio::task::spawn(async move {
            let _ = write_sst(offset, bytes);
        });

        Ok(())
    }
}

fn write_sst(offset: usize, bytes: Vec<u8>) -> Result<(), Error> {
    let path = format!("ontos/{}.sst", offset);
    let mut file = File::create(path)?;
    file.write_all(&bytes)?;
    Ok(())
}

#[cfg(test)]
mod test {
    use crate::db::Entry;

    use super::*;

    #[tokio::test]
    async fn memtable_capacity() {
        let mut driver = Driver {
            master: MemTable::with_capacity(10),
            offset: 0,
        };

        for i in 0..10 {
            driver.write(i.to_string(), i).await.unwrap();
        }

        assert!(driver.master.at_capacity());

        driver.write(String::from("11"), 11).await.unwrap();

        assert_eq!(
            driver.master.items(),
            vec![Entry {
                key: String::from("11"),
                value: 11,
            }]
        )
    }
}
