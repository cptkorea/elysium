mod table;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct KeyValue {
    pub key: String,
    pub value: u32,
}
