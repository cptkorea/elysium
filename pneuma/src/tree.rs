use std::cmp::Ordering;
use std::fmt::Debug;

use thiserror::Error;

pub mod avl;
pub mod bst;
pub mod iter;

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
