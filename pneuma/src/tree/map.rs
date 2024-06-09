use super::bst::BinarySearchTree;

#[derive(Debug)]
pub(crate) struct MapKey<K: Ord> {
    pub key: K,
    pub index: usize,
}

impl<K: Ord> Eq for MapKey<K> {}

impl<K: Ord> PartialEq for MapKey<K> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl<K: Ord> Ord for MapKey<K> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

impl<K: Ord> PartialOrd for MapKey<K> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.key.cmp(&other.key))
    }
}

pub struct BSTMap<K: Ord, V> {
    keys: BinarySearchTree<MapKey<K>>,
    values: Vec<V>,
}

impl<K: Ord, V> BSTMap<K, V> {
    pub fn new() -> Self {
        Self {
            keys: BinarySearchTree::new(),
            values: Vec::with_capacity(100),
        }
    }

    pub fn set(&mut self, key: K, value: V) {
        self.keys.insert(MapKey {
            key,
            index: self.values.len(),
        });
        self.values.push(value);
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.keys
            .find_node(|m, k| m.key.cmp(k), key)
            .map_or(None, |n| self.values.get(n.item.index))
    }
}
