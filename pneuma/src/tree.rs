use std::cmp::Ordering;
use std::collections::VecDeque;
use std::fmt::Debug;

use thiserror::Error;

pub mod avl;

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
pub struct BinaryTree<T: Ord> {
    root: Option<BoxedNode<T>>,
    size: usize,
}

impl<T: Ord> BinaryTree<T> {
    pub fn new() -> Self {
        Self {
            root: None,
            size: 0,
        }
    }

    pub fn height(&self) -> i32 {
        self.root.as_ref().map_or(0, |r| r.height)
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn insert(&mut self, value: T) -> Result<(), Error> {
        match self.root.as_mut() {
            None => {
                self.root = Some(BinaryTreeNode::create(value));
            }
            Some(t) => {
                t.insert(value)?;
            }
        }
        self.size += 1;
        Ok(())
    }

    pub fn contains(&self, value: T) -> bool {
        self.root
            .as_ref()
            .map(|r| r.find(value).is_some())
            .unwrap_or_default()
    }

    pub fn iter(&self) -> TreeRefIterator<'_, T> {
        self.into_iter()
    }
}

pub struct BinaryTreeNode<T: Ord> {
    value: T,
    height: i32,
    left: Option<BoxedNode<T>>,
    right: Option<BoxedNode<T>>,
}

impl<T: Ord> BinaryTreeNode<T> {
    fn create(value: T) -> BoxedNode<T> {
        Box::new(Self {
            value,
            height: 0,
            left: None,
            right: None,
        })
    }

    fn create_child(&mut self, value: T, orientation: Orientation) {
        let node: BoxedNode<T> = BinaryTreeNode::create(value);
        match orientation {
            Orientation::Left => self.left = Some(node),
            Orientation::Right => self.right = Some(node),
        }
    }

    fn find(&self, value: T) -> Option<&BinaryTreeNode<T>> {
        match value.cmp(&self.value) {
            Ordering::Less => match self.left.as_ref() {
                Some(left) => left.find(value),
                None => None,
            },
            Ordering::Greater => match self.right.as_ref() {
                Some(right) => right.find(value),
                None => None,
            },
            Ordering::Equal => return Some(self),
        }
    }

    fn insert(&mut self, value: T) -> Result<(), Error> {
        match value.cmp(&self.value) {
            Ordering::Less => {
                match self.left.as_mut() {
                    Some(left) => left.insert(value)?,
                    None => self.create_child(value, Orientation::Left),
                }
                self.update_height();
            }
            Ordering::Greater => {
                match self.right.as_mut() {
                    Some(right) => right.insert(value)?,
                    None => self.create_child(value, Orientation::Right),
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
            &next.value
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
            next.value
        })
    }
}

impl<'a, T: Ord> IntoIterator for &'a BinaryTree<T> {
    type Item = &'a T;
    type IntoIter = TreeRefIterator<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        TreeRefIterator {
            curr: self.root.as_ref(),
            queue: VecDeque::with_capacity(10),
        }
    }
}

impl<T: Ord> IntoIterator for BinaryTree<T> {
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

    fn insert_node<T: Ord + Debug>(tree: &mut BinaryTree<T>, value: T) {
        tree.insert(value).expect("unable to insert node");
    }

    #[test]
    fn insert_nodes() {
        let mut tree = BinaryTree::new();
        assert_eq!(0, tree.size());

        insert_node(&mut tree, 1);
        assert_eq!(1, tree.size());

        insert_node(&mut tree, 2);
        assert_eq!(2, tree.size());
    }

    #[test]
    fn contains() {
        let mut tree = BinaryTree::new();
        let values = [2, 1, 3, 4];

        for v in values {
            insert_node(&mut tree, v);
        }

        for v in values {
            assert!(tree.contains(v));
        }

        assert!(!tree.contains(0));
        assert!(!tree.contains(5));
    }

    #[test]
    fn height() {
        let mut tree = BinaryTree::new();

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
        let mut tree = BinaryTree::new();
        let insertions = [3, 4, 5, 2, 1, 7, 6];

        for i in insertions {
            insert_node(&mut tree, i);
        }

        let nodes: Vec<&u32> = tree.iter().collect();
        assert_eq!(vec![&1, &2, &3, &4, &5, &6, &7], nodes);
    }

    #[test]
    fn into_iterator() {
        let mut tree = BinaryTree::new();
        let insertions = [3, 4, 5, 2, 1, 7, 6];

        for i in insertions {
            insert_node(&mut tree, i);
        }

        let nodes: Vec<u32> = tree.into_iter().collect();
        assert_eq!(vec![1, 2, 3, 4, 5, 6, 7], nodes);
    }
}
