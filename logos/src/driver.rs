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
        Ok(self.master.write(key, value)?)
    }

    pub async fn flush_table(&mut self) -> Result<(), Error> {
        let path = get_path(self.offset);

        let sst = SSTable::from(&self.master);
        let bytes = sst.into_bytes()?;

        let mut file = File::create(path)?;
        file.write_all(&bytes)?;
        Ok(())
    }
}

fn get_path(offset: usize) -> String {
    format!("logos/{}.sst", offset)
}
