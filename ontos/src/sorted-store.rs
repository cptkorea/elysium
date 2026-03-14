use std::borrow::Borrow;
use std::collections::BTreeMap;

use elysium_common::skiplist::{Iter as SkipListIter, RandomN, SkipList};

/// A simple sorted key-value storage interface used by `MemTable` that can
/// be swapped through configuration.
///
/// Implementors must keep keys in sorted order and support:
/// - owned inserts (`insert`)
/// - borrowed lookups (`get`)
/// - sorted iteration (`iter_sorted`)
/// - clearing all entries (`clear`)
pub trait SortedStore<K: Ord, V> {
    type Iter<'a>: Iterator<Item = (&'a K, &'a V)>
    where
        Self: 'a,
        K: 'a,
        V: 'a;

    fn insert(&mut self, key: K, value: V);
    fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized;
    fn iter_sorted(&self) -> Self::Iter<'_>;
    fn clear(&mut self);
}

impl<K: Ord, V> SortedStore<K, V> for BTreeMap<K, V> {
    type Iter<'a>
        = std::collections::btree_map::Iter<'a, K, V>
    where
        Self: 'a,
        K: 'a,
        V: 'a;

    fn insert(&mut self, key: K, value: V) {
        BTreeMap::insert(self, key, value);
    }

    fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        BTreeMap::get(self, key)
    }

    fn iter_sorted(&self) -> Self::Iter<'_> {
        self.iter()
    }

    fn clear(&mut self) {
        BTreeMap::clear(self);
    }
}

impl<K: Ord, V, R: RandomN> SortedStore<K, V> for SkipList<K, V, R> {
    type Iter<'a>
        = SkipListIter<'a, K, V, R>
    where
        Self: 'a,
        K: 'a,
        V: 'a;

    fn insert(&mut self, key: K, value: V) {
        let _ = SkipList::insert(self, key, value);
    }

    fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        SkipList::get(self, key)
    }

    fn iter_sorted(&self) -> Self::Iter<'_> {
        SkipList::iter(self)
    }

    fn clear(&mut self) {
        SkipList::clear(self);
    }
}
