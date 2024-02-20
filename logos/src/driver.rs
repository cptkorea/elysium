use std::fs::File;
use std::io::Write;

use crate::db::{MemTable, SSTable};
use crate::Error;

pub struct Driver {
    master: MemTable,
    offset: usize,
}

impl Driver {
    pub fn new() -> Self {
        Self {
            master: MemTable::new(),
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
    let path = format!("logos/{}.sst", offset);
    let mut file = File::create(path)?;
    file.write_all(&bytes)?;
    Ok(())
}
