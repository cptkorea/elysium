//! A probabilistic skiplist providing O(log n) average-case search, insertion,
//! and deletion.
//!
//! A skiplist is a layered sorted linked list where higher levels act as
//! "express lanes," allowing traversal to skip over large sections of the
//! lower levels. This gives performance comparable to a balanced binary
//! search tree while being simpler to implement.
//! 
//! https://en.wikipedia.org/wiki/Skip_list
//! 
//! # Implementation Notes
//! The implementation is based on the paper ["Skip Lists: A Probabilistic Alternative to Balanced Trees"]
//! (https://www.cl.cam.ac.uk/teaching/0506/Algorithms/skiplists.pdf) by William Pugh.
//!
//! # Architecture
//!
//! This implementation uses an **arena-based** layout: all nodes live in a
//! contiguous `Vec`, and forward pointers are indices rather than heap
//! pointers. This avoids `Rc<RefCell<>>` overhead and all `unsafe` code.
//!
//! # Randomization
//!
//! The level assigned to each new node is determined by a [`RandomN`]
//! implementation. The default, [`XorShift`], is a minimal xorshift64 PRNG
//! that requires no external dependencies. Custom generators (e.g. backed by
//! the `rand` crate) can be injected via [`SkipList::with_rng`].
//!
//! # Quick Start
//!
//! ```
//! use common::skiplist::SkipList;
//!
//! let mut list = SkipList::new();
//! list.insert(3, "three");
//! list.insert(1, "one");
//!
//! assert_eq!(list.get(&1), Some(&"one"));
//! assert_eq!(list.get(&2), None);
//! ```

use std::borrow::Borrow;
use std::fmt;
use std::mem::size_of;

pub use crate::rng::{RandomN, XorShift};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default maximum number of levels in the skiplist.
///
/// 16 levels can efficiently handle up to 2^16 = 65,536 elements with the
/// default probability of 0.5. For larger collections, pass a higher value
/// to [`SkipList::with_max_level`] or [`SkipList::with_rng`].
///
/// The relationship between `max_level` and optimal capacity is approximately:
///
/// ```text
/// optimal_n ≈ (1/p)^max_level
/// ```
///
/// where `p` is the promotion probability (default 0.5).
pub const DEFAULT_MAX_LEVEL: usize = 16;

// ---------------------------------------------------------------------------
// Node (internal)
// ---------------------------------------------------------------------------

/// A single node in the skiplist arena.
///
/// Stores a key-value pair and a tower of forward pointers whose height is
/// determined at insertion time by the [`RandomN`] generator.
struct Node<K, V> {
    key: K,
    value: V,
    /// Forward pointers, one per level this node participates in.
    /// `forward[i]` is `Some(arena_index)` pointing to the next node at
    /// level `i`, or `None` if this is the last node at that level.
    forward: Vec<Option<usize>>,
}

// ---------------------------------------------------------------------------
// SkipList
// ---------------------------------------------------------------------------

/// A probabilistic sorted map providing O(log n) average-case operations.
///
/// # Type Parameters
///
/// - `K` — Key type. Must implement [`Ord`] for sorted ordering.
/// - `V` — Value type.
/// - `R` — Randomization strategy. Must implement [`RandomN`]. Defaults to
///   [`XorShift`], a built-in xorshift64 PRNG. Swap in a custom generator
///   via [`SkipList::with_rng`].
///
/// # Initialization
///
/// There are three ways to create a skiplist, in order of increasing control:
///
/// - [`SkipList::new`] — max_level=16, [`XorShift`] with p=0.5
/// - [`SkipList::with_max_level`] — custom level cap, default [`XorShift`]
/// - [`SkipList::with_rng`] — custom level cap and custom [`RandomN`] impl
///
/// **Choosing `max_level`:** a good rule of thumb is `log₂(expected_n)`.
/// The default of 16 handles ~65K elements well. For a million elements,
/// use 20.
///
/// # Memory Layout
///
/// Nodes are stored in a contiguous `Vec` (arena). Forward pointers are
/// indices into this arena, avoiding `Rc<RefCell<>>` and all `unsafe` code.
/// Removed nodes are recycled via an internal free list.
///
/// # Examples
///
/// ```
/// use common::skiplist::SkipList;
///
/// let mut sl = SkipList::new();
/// sl.insert("banana", 2);
/// sl.insert("apple", 1);
/// sl.insert("cherry", 3);
///
/// assert_eq!(sl.get("apple"), Some(&1));
/// assert_eq!(sl.len(), 3);
///
/// sl.remove("banana");
/// assert_eq!(sl.get("banana"), None);
/// assert_eq!(sl.len(), 2);
/// ```
pub struct SkipList<K, V, R: RandomN = XorShift> {
    /// Forward pointers for the head sentinel, one per possible level.
    /// `head[i]` points to the first node at level `i`, or `None` if that
    /// level is empty. Length is always equal to `max_level`.
    head: Vec<Option<usize>>,

    /// Arena holding all nodes. Slots are `None` when a node has been removed
    /// and is awaiting reuse.
    arena: Vec<Option<Node<K, V>>>,

    /// Indices of removed (vacant) arena slots available for reuse.
    free_list: Vec<usize>,

    /// Maximum number of levels this skiplist supports. Fixed at construction.
    max_level: usize,

    /// Current highest level containing at least one node (0-indexed).
    /// Starts at 0 and grows as taller nodes are inserted.
    level: usize,

    /// The pluggable random number generator determining new-node heights.
    rng: R,

    /// Number of key-value pairs currently stored.
    len: usize,

    /// Reusable scratch buffer for predecessor tracking during insert/remove.
    /// Allocated once at construction and cleared before each use, avoiding a
    /// heap allocation on every mutating operation.
    update_buf: Vec<Option<usize>>,
}

// -- Convenience constructors (XorShift default) ----------------------------

impl<K: Ord, V> SkipList<K, V, XorShift> {
    /// Creates a new skiplist with recommended defaults.
    ///
    /// - **Max level:** 16 — suitable for up to ~65K elements
    /// - **Probability:** 0.5 — each level is roughly half the size below
    /// - **RNG:** [`XorShift`] seeded from the system clock
    ///
    /// This is the simplest way to get started:
    ///
    /// ```
    /// use common::skiplist::SkipList;
    ///
    /// let mut sl: SkipList<i32, &str> = SkipList::new();
    /// sl.insert(1, "hello");
    /// ```
    pub fn new() -> Self {
        Self::with_max_level(DEFAULT_MAX_LEVEL)
    }

    /// Creates a new skiplist with a custom maximum level and default RNG.
    ///
    /// Use this when you know the approximate dataset size and want to tune
    /// the level count. A good heuristic is `log₂(n)` where `n` is the
    /// expected number of elements.
    ///
    /// # Panics
    ///
    /// Panics if `max_level` is 0.
    ///
    /// # Examples
    ///
    /// ```
    /// use common::skiplist::SkipList;
    ///
    /// // Optimized for up to ~1 million elements
    /// let sl: SkipList<u64, String> = SkipList::with_max_level(20);
    /// ```
    pub fn with_max_level(max_level: usize) -> Self {
        Self::with_rng(max_level, XorShift::default())
    }
}

impl<K: Ord, V> Default for SkipList<K, V, XorShift> {
    fn default() -> Self {
        Self::new()
    }
}

// -- Core implementation (generic over R) -----------------------------------

impl<K: Ord, V, R: RandomN> SkipList<K, V, R> {
    /// Creates a new skiplist with a custom maximum level and RNG.
    ///
    /// This is the most flexible constructor, giving full control over both
    /// the skiplist's height and its randomization strategy.
    ///
    /// # Panics
    ///
    /// Panics if `max_level` is 0.
    ///
    /// # Examples
    ///
    /// ```
    /// use common::skiplist::XorShift;
    /// use common::skiplist::SkipList;
    ///
    /// // Lower probability = flatter lists (more nodes at level 0, fewer express lanes)
    /// let rng = XorShift::with_seed(0.25, 12345);
    /// let mut sl: SkipList<i32, i32, XorShift> = SkipList::with_rng(8, rng);
    /// sl.insert(1, 100);
    /// ```
    pub fn with_rng(max_level: usize, rng: R) -> Self {
        assert!(max_level > 0, "max_level must be at least 1");
        Self {
            head: vec![None; max_level],
            arena: Vec::new(),
            free_list: Vec::new(),
            max_level,
            level: 0,
            rng,
            len: 0,
            update_buf: vec![None; max_level],
        }
    }

    /// Returns the number of key-value pairs in the skiplist.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the skiplist contains no elements.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns `true` if the skiplist contains the given key.
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.get(key).is_some()
    }

    /// Returns the configured maximum level for this skiplist.
    pub fn max_level(&self) -> usize {
        self.max_level
    }

    /// Returns a reference to the value associated with `key`, or `None` if
    /// the key is not present.
    ///
    /// Traversal begins at the highest active level and drops down, skipping
    /// over nodes with smaller keys at each level before descending.
    ///
    /// # Time Complexity
    ///
    /// O(log n) on average.
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let mut current: Option<usize> = None;

        for i in (0..=self.level).rev() {
            loop {
                let next = self.forward_at(current, i);
                match next {
                    Some(idx) if self.node(idx).key.borrow() < key => current = Some(idx),
                    _ => break,
                }
            }
        }

        let candidate_idx = self.forward_at(current, 0)?;
        let candidate = self.node(candidate_idx);
        if candidate.key.borrow() == key {
            Some(&candidate.value)
        } else {
            None
        }
    }

    /// Inserts a key-value pair into the skiplist.
    ///
    /// If the key already exists, its value is updated in place and the
    /// previous value is returned. Otherwise, a new node is allocated with
    /// a random tower height and `None` is returned.
    ///
    /// # Time Complexity
    ///
    /// O(log n) on average.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        // Reset the scratch buffer instead of allocating a new Vec.
        self.update_buf.iter_mut().for_each(|slot| *slot = None);

        let mut current: Option<usize> = None;

        for i in (0..=self.level).rev() {
            loop {
                let next = self.forward_at(current, i);
                match next {
                    Some(idx) if self.node(idx).key < key => current = Some(idx),
                    _ => break,
                }
            }
            self.update_buf[i] = current;
        }

        // If the key already exists at level 0, update its value in place.
        if let Some(idx) = self.forward_at(current, 0) {
            if self.node(idx).key == key {
                let old = std::mem::replace(&mut self.node_mut(idx).value, value);
                return Some(old);
            }
        }

        let new_level = self.rng.random_n(self.max_level);

        // If the new node is taller than any existing node, the extra levels
        // have the head sentinel as their predecessor (already None from reset).
        if new_level > self.level {
            self.level = new_level;
        }

        let node = Node {
            key,
            value,
            forward: vec![None; new_level + 1],
        };
        let node_idx = self.alloc_node(node);

        // Wire the new node into each level it participates in by splicing
        // it between its predecessor and successor.
        for i in 0..=new_level {
            let prev_next = self.forward_at(self.update_buf[i], i);
            self.node_mut(node_idx).forward[i] = prev_next;
            self.set_forward(self.update_buf[i], i, Some(node_idx));
        }

        self.len += 1;
        None
    }

    /// Removes the entry for `key` and returns its value, or `None` if the
    /// key was not found.
    ///
    /// The vacated arena slot is added to an internal free list and will be
    /// reused by subsequent inserts.
    ///
    /// # Time Complexity
    ///
    /// O(log n) on average.
    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.update_buf.iter_mut().for_each(|slot| *slot = None);

        let mut current: Option<usize> = None;

        for i in (0..=self.level).rev() {
            // At each level, walk forward as far as possible while keys are less than the target
            while let Some(idx) = self.forward_at(current, i) {
                if self.node(idx).key.borrow() < key {
                    current = Some(idx);
                } else {
                    break;
                }
            }
            self.update_buf[i] = current;
        }

        let target_idx = self.forward_at(current, 0)?;
        if self.node(target_idx).key.borrow() != key {
            return None;
        }

        // Unlink the target node from every level it participates in.
        let target_height = self.node(target_idx).forward.len();
        for i in 0..target_height {
            let target_next = self.node(target_idx).forward[i];
            self.set_forward(self.update_buf[i], i, target_next);
        }

        // Shrink the active level if the top levels are now empty.
        while self.level > 0 && self.head[self.level].is_none() {
            self.level -= 1;
        }

        let removed = self.arena[target_idx].take().expect("node was already freed");
        self.free_list.push(target_idx);
        self.len -= 1;
        Some(removed.value)
    }

    // -- Private helpers ----------------------------------------------------

    /// Returns the forward pointer at `level` for the given position.
    /// `None` as `pos` represents the head sentinel.
    fn forward_at(&self, pos: Option<usize>, level: usize) -> Option<usize> {
        match pos {
            None => self.head[level],
            Some(idx) => self.node(idx).forward[level],
        }
    }

    /// Sets the forward pointer at `level` for the given position.
    fn set_forward(&mut self, pos: Option<usize>, level: usize, target: Option<usize>) {
        match pos {
            None => self.head[level] = target,
            Some(idx) => self.node_mut(idx).forward[level] = target,
        }
    }

    /// Returns a shared reference to the node at `idx` in the arena.
    fn node(&self, idx: usize) -> &Node<K, V> {
        self.arena[idx].as_ref().expect("accessed a freed node")
    }

    /// Returns a mutable reference to the node at `idx` in the arena.
    fn node_mut(&mut self, idx: usize) -> &mut Node<K, V> {
        self.arena[idx].as_mut().expect("accessed a freed node")
    }

    /// Allocates a slot for `node`, reusing a previously freed slot when
    /// available to avoid unbounded arena growth.
    fn alloc_node(&mut self, node: Node<K, V>) -> usize {
        if let Some(idx) = self.free_list.pop() {
            self.arena[idx] = Some(node);
            idx
        } else {
            let idx = self.arena.len();
            self.arena.push(Some(node));
            idx
        }
    }

        /// Returns an estimate of the total heap memory (in bytes) owned by this
    /// skiplist.
    ///
    /// Includes the arena, every node's forward-pointer array, the head and
    /// scratch buffers, and the free list. Does **not** account for heap
    /// memory owned by `K` or `V` themselves (e.g. the backing buffer of a
    /// `String` key).
    ///
    /// This is an O(n) operation — intended for diagnostics and testing, not
    /// for use in hot paths.
    pub fn heap_size(&self) -> usize {
        let head = self.head.capacity() * size_of::<Option<usize>>();
        let update_buf = self.update_buf.capacity() * size_of::<Option<usize>>();
        let free_list = self.free_list.capacity() * size_of::<usize>();

        let arena_shell = self.arena.capacity() * size_of::<Option<Node<K, V>>>();
        let arena_forward: usize = self
            .arena
            .iter()
            .filter_map(|slot| slot.as_ref())
            .map(|node| node.forward.capacity() * size_of::<Option<usize>>())
            .sum();

        head + update_buf + free_list + arena_shell + arena_forward
    }
}

// -- Debug ------------------------------------------------------------------

impl<K: fmt::Debug + Ord, V: fmt::Debug, R: RandomN> fmt::Debug for SkipList<K, V, R> {
    /// Formats the skiplist as a sorted debug map by traversing level 0.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = f.debug_map();
        let mut current = self.head[0];
        while let Some(idx) = current {
            let node = self.arena[idx].as_ref().expect("freed node in list");
            map.entry(&node.key, &node.value);
            current = node.forward[0];
        }
        map.finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic generator that always returns the same level.
    struct FixedLevel(usize);

    impl RandomN for FixedLevel {
        fn random_n(&mut self, max: usize) -> usize {
            self.0.min(max - 1)
        }
    }

    /// Helper: creates a skiplist with a deterministic seed for reproducible tests.
    fn seeded_list() -> SkipList<i32, &'static str, XorShift> {
        let rng = XorShift::with_seed(0.5, 42);
        SkipList::with_rng(DEFAULT_MAX_LEVEL, rng)
    }

    #[test]
    fn empty_list() {
        let sl: SkipList<i32, i32> = SkipList::default();
        assert!(sl.is_empty());
        assert_eq!(sl.len(), 0);
        assert_eq!(sl.get(&1), None);
    }

    #[test]
    fn insert_and_get() {
        let mut sl = seeded_list();
        assert_eq!(sl.insert(3, "three"), None);
        assert_eq!(sl.insert(1, "one"), None);
        assert_eq!(sl.insert(2, "two"), None);

        assert_eq!(sl.get(&1), Some(&"one"));
        assert_eq!(sl.get(&2), Some(&"two"));
        assert_eq!(sl.get(&3), Some(&"three"));
        assert_eq!(sl.get(&4), None);
        assert_eq!(sl.len(), 3);
    }

    #[test]
    fn update_existing_key() {
        let mut sl = seeded_list();
        sl.insert(1, "one");
        assert_eq!(sl.insert(1, "uno"), Some("one"));
        assert_eq!(sl.get(&1), Some(&"uno"));
        assert_eq!(sl.len(), 1);
    }

    #[test]
    fn remove_existing() {
        let mut sl = seeded_list();
        sl.insert(1, "one");
        sl.insert(2, "two");
        sl.insert(3, "three");

        assert_eq!(sl.remove(&2), Some("two"));
        assert_eq!(sl.get(&2), None);
        assert_eq!(sl.len(), 2);
        assert_eq!(sl.get(&1), Some(&"one"));
        assert_eq!(sl.get(&3), Some(&"three"));
    }

    #[test]
    fn remove_nonexistent() {
        let mut sl = seeded_list();
        sl.insert(1, "one");
        assert_eq!(sl.remove(&99), None);
        assert_eq!(sl.len(), 1);
    }

    #[test]
    fn contains_key_check() {
        let mut sl = seeded_list();
        sl.insert(10, "ten");
        assert!(sl.contains_key(&10));
        assert!(!sl.contains_key(&20));
    }

    #[test]
    fn many_inserts() {
        let rng = XorShift::with_seed(0.5, 123);
        let mut sl: SkipList<i32, i32, XorShift> = SkipList::with_rng(20, rng);

        for i in (0..1000).rev() {
            sl.insert(i, i * 10);
        }

        assert_eq!(sl.len(), 1000);
        for i in 0..1000 {
            assert_eq!(sl.get(&i), Some(&(i * 10)));
        }
    }

    #[test]
    fn insert_remove_reuse() {
        let mut sl = seeded_list();
        sl.insert(1, "a");
        sl.insert(2, "b");
        sl.remove(&1);
        sl.insert(3, "c");

        assert_eq!(sl.get(&1), None);
        assert_eq!(sl.get(&2), Some(&"b"));
        assert_eq!(sl.get(&3), Some(&"c"));
        assert_eq!(sl.len(), 2);
    }

    #[test]
    fn custom_level_generator() {
        let mut sl: SkipList<i32, i32, FixedLevel> = SkipList::with_rng(4, FixedLevel(0));
        sl.insert(1, 10);
        sl.insert(2, 20);
        assert_eq!(sl.get(&1), Some(&10));
        assert_eq!(sl.get(&2), Some(&20));
    }

    #[test]
    fn debug_output_is_sorted() {
        let mut sl = seeded_list();
        sl.insert(3, "c");
        sl.insert(1, "a");
        sl.insert(2, "b");
        let debug = format!("{:?}", sl);
        assert!(debug.contains("1: \"a\""));
        assert!(debug.contains("2: \"b\""));
        assert!(debug.contains("3: \"c\""));
    }

    #[test]
    fn heap_size_grows_with_inserts() {
        let mut sl = seeded_list();
        let empty_size = sl.heap_size();
        assert!(empty_size > 0, "even an empty list owns heap buffers");

        sl.insert(1, "one");
        let one_size = sl.heap_size();
        assert!(one_size > empty_size);

        for i in 2..=100 {
            sl.insert(i, "x");
        }
        let full_size = sl.heap_size();
        assert!(full_size > one_size);
    }

    #[test]
    fn max_level_accessor() {
        let sl: SkipList<i32, i32> = SkipList::with_max_level(8);
        assert_eq!(sl.max_level(), 8);

        let sl2: SkipList<i32, i32> = SkipList::new();
        assert_eq!(sl2.max_level(), DEFAULT_MAX_LEVEL);
    }
}
