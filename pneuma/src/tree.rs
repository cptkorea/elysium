use std::cmp::Ordering;
use std::collections::VecDeque;
use std::fmt::Debug;

use thiserror::Error;

pub mod avl;
pub mod map;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Tree insertion error")]
    InsertionError,
}

type BoxedNode<T> = Box<BinaryTreeNode<T>>;

#[derive(Debug)]
pub enum Orientation {
    Left,
    Right,
}

#[derive(Default)]
pub struct BinarySearchTree<T: Ord> {
    root: Option<BoxedNode<T>>,
    size: usize,
}

impl<T: Ord> BinarySearchTree<T> {
    pub fn new() -> Self {
        Self {
            root: None,
            size: 0,
        }
    }

    fn height(&self) -> i32 {
        self.root.as_ref().map_or(0, |r| r.height)
    }

    fn size(&self) -> usize {
        self.size
    }

    fn insert(&mut self, item: T) -> Result<(), Error> {
        match self.root.as_mut() {
            None => {
                self.root = Some(BinaryTreeNode::create(item));
            }
            Some(t) => {
                t.insert(item)?;
            }
        }
        self.size += 1;
        Ok(())
    }

    fn contains(&self, item: &T) -> bool {
        self.root
            .as_ref()
            .map(|r| r.find(&item).is_some())
            .unwrap_or_default()
    }

    pub fn iter(&self) -> TreeRefIterator<'_, T> {
        self.into_iter()
    }

    pub fn level_iter(&self) -> LevelIterator<T> {
        LevelIterator {
            curr: self.root.as_ref(),
            queue: VecDeque::with_capacity(10),
        }
    }
}

pub struct BinaryTreeNode<T: Ord> {
    item: T,
    height: i32,
    left: Option<BoxedNode<T>>,
    right: Option<BoxedNode<T>>,
}

impl<T: Ord> BinaryTreeNode<T> {
    fn create(item: T) -> BoxedNode<T> {
        Box::new(Self {
            item,
            height: 0,
            left: None,
            right: None,
        })
    }

    fn create_child(&mut self, item: T, orientation: Orientation) {
        let node: BoxedNode<T> = BinaryTreeNode::create(item);
        match orientation {
            Orientation::Left => self.left = Some(node),
            Orientation::Right => self.right = Some(node),
        }
    }

    fn find(&self, item: &T) -> Option<&BinaryTreeNode<T>> {
        match item.cmp(&self.item) {
            Ordering::Less => match self.left.as_ref() {
                Some(left) => left.find(item),
                None => None,
            },
            Ordering::Greater => match self.right.as_ref() {
                Some(right) => right.find(item),
                None => None,
            },
            Ordering::Equal => return Some(self),
        }
    }

    fn insert(&mut self, item: T) -> Result<(), Error> {
        match item.cmp(&self.item) {
            Ordering::Less => {
                match self.left.as_mut() {
                    Some(left) => left.insert(item)?,
                    None => self.create_child(item, Orientation::Left),
                }
                self.update_height();
            }
            Ordering::Greater => {
                match self.right.as_mut() {
                    Some(right) => right.insert(item)?,
                    None => self.create_child(item, Orientation::Right),
                }
                self.update_height();
            }
            Ordering::Equal => return Err(Error::InsertionError),
        }
        Ok(())
    }

    fn update_height(&mut self) {
        let (lh, rh) = self.child_heights();
        self.height = 1 + std::cmp::max(lh, rh);
    }

    fn child_heights(&self) -> (i32, i32) {
        (
            self.left.as_ref().map_or(-1, |l| l.height),
            self.right.as_ref().map_or(-1, |l| l.height),
        )
    }
}

pub struct LevelIterator<'a, T: Ord> {
    curr: Option<&'a BoxedNode<T>>,
    queue: VecDeque<&'a BoxedNode<T>>,
}

pub struct TreeRefIterator<'a, T: Ord> {
    curr: Option<&'a BoxedNode<T>>,
    queue: VecDeque<&'a BoxedNode<T>>,
}

pub struct TreeIterator<T: Ord> {
    curr: Option<BoxedNode<T>>,
    queue: VecDeque<BoxedNode<T>>,
}

impl<'a, T: Ord> Iterator for TreeRefIterator<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(next) = self.curr {
            self.curr = next.left.as_ref();
            self.queue.push_front(next);
        }

        self.queue.pop_front().map(|next| {
            self.curr = next.right.as_ref();
            &next.item
        })
    }
}

impl<T: Ord> Iterator for TreeIterator<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(mut next) = self.curr.take() {
            self.curr = next.left.take();
            self.queue.push_front(next);
        }

        self.queue.pop_front().map(|mut next| {
            self.curr = next.right.take();
            next.item
        })
    }
}

impl<'a, T: Ord> Iterator for LevelIterator<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.curr.take().map(|n| {
            if let Some(left) = n.left.as_ref() {
                self.queue.push_back(left);
            }

            if let Some(right) = n.right.as_ref() {
                self.queue.push_back(right);
            }
            &n.item
        });

        self.curr = self.queue.pop_front();
        next
    }
}

impl<'a, T: Ord> IntoIterator for &'a BinarySearchTree<T> {
    type Item = &'a T;
    type IntoIter = TreeRefIterator<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        TreeRefIterator {
            curr: self.root.as_ref(),
            queue: VecDeque::with_capacity(10),
        }
    }
}

impl<T: Ord> IntoIterator for BinarySearchTree<T> {
    type Item = T;
    type IntoIter = TreeIterator<T>;

    fn into_iter(self) -> Self::IntoIter {
        TreeIterator {
            curr: self.root,
            queue: VecDeque::with_capacity(10),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn insert_node<T: Ord + Debug>(tree: &mut BinarySearchTree<T>, item: T) {
        tree.insert(item).expect("unable to insert node");
    }

    #[test]
    fn insert_nodes() {
        let mut tree = BinarySearchTree::new();
        assert_eq!(0, tree.size());

        insert_node(&mut tree, 1);
        assert_eq!(1, tree.size());

        insert_node(&mut tree, 2);
        assert_eq!(2, tree.size());
    }

    #[test]
    fn contains() {
        let mut tree = BinarySearchTree::new();
        let values = [2, 1, 3, 4];

        for v in values {
            insert_node(&mut tree, v);
        }

        for v in values {
            assert!(tree.contains(&v));
        }

        assert!(!tree.contains(&0));
        assert!(!tree.contains(&5));
    }

    #[test]
    fn height() {
        let mut tree = BinarySearchTree::new();

        insert_node(&mut tree, 2);
        assert_eq!(0, tree.height());
        insert_node(&mut tree, 1);
        assert_eq!(1, tree.height());
        insert_node(&mut tree, 3);
        assert_eq!(1, tree.height());
        insert_node(&mut tree, 4);
        assert_eq!(2, tree.height());
    }

    #[test]
    fn iterator() {
        let mut tree = BinarySearchTree::new();
        let insertions = [3, 4, 5, 2, 1, 7, 6];

        for i in insertions {
            insert_node(&mut tree, i);
        }

        let nodes: Vec<&u32> = tree.iter().collect();
        assert_eq!(vec![&1, &2, &3, &4, &5, &6, &7], nodes);
    }

    #[test]
    fn into_iterator() {
        let mut tree = BinarySearchTree::new();
        let insertions = [3, 4, 5, 2, 1, 7, 6];

        for i in insertions {
            insert_node(&mut tree, i);
        }

        let nodes: Vec<u32> = tree.into_iter().collect();
        assert_eq!(vec![1, 2, 3, 4, 5, 6, 7], nodes);
    }

    #[test]
    fn level_iterator() {
        let mut tree = BinarySearchTree::new();

        insert_node(&mut tree, 2);
        insert_node(&mut tree, 3);
        insert_node(&mut tree, 1);

        let mut level_iter = tree.level_iter();
        assert_eq!(Some(&2), level_iter.next());
        assert_eq!(Some(&1), level_iter.next());
        assert_eq!(Some(&3), level_iter.next());
        assert_eq!(None, level_iter.next());
        assert_eq!(None, level_iter.next());
    }
}
