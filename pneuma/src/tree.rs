use std::cmp::Ordering;
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
