use super::{bst::BinarySearchTree, BinaryTreeNode, Error, Orientation};
use std::cmp::Ordering;
use std::fmt::Debug;

#[cfg(test)]
use super::iter::{LevelIter, NodeIter};

const BALANCE_THRESHOLD: i32 = 1;

#[derive(Default)]
pub struct AVLTree<T: Ord> {
    inner: BinarySearchTree<T>,
}

impl<T: Ord> AVLTree<T> {
    pub fn new() -> Self {
        Self {
            inner: BinarySearchTree::new(),
        }
    }

    pub fn size(&self) -> usize {
        self.inner.size()
    }

    pub fn insert(&mut self, value: T) -> Result<(), Error> {
        self.inner.insert_with_fn(value, AVLNode::insert)
    }

    pub fn contains(&self, value: &T) -> bool {
        self.inner.contains(value)
    }

    #[cfg(test)]
    fn is_balanced(&self) -> bool {
        self.nodes_iter()
            .all(|n| AVLNode::balance(n.as_ref()) == Balance::Balanced)
    }

    #[cfg(test)]
    fn level_iter(&self) -> LevelIter<'_, T> {
        self.inner.level_iter()
    }

    #[cfg(test)]
    fn nodes_iter(&self) -> NodeIter<'_, T> {
        self.inner.nodes_iter()
    }

    #[cfg(test)]
    fn height(&self) -> i32 {
        self.inner.height()
    }
}

trait AVLNode<T: Ord> {
    const THRESHOLD: i32;

    fn balance(&self) -> Balance;
    fn insert(&mut self, value: T) -> Result<(), Error>;
    fn rotate(&mut self, direction: Orientation) -> Result<(), Error>;
}

impl<T: Ord> AVLNode<T> for BinaryTreeNode<T> {
    const THRESHOLD: i32 = BALANCE_THRESHOLD;

    fn balance(&self) -> Balance {
        let (lh, rh) = self.child_heights();
        match lh - rh {
            i32::MIN..=-2 => Balance::RightHeavy,
            -1..=1 => Balance::Balanced,
            2..=i32::MAX => Balance::LeftHeavy,
        }
    }

    fn insert(&mut self, value: T) -> Result<(), Error> {
        match value.cmp(&self.item) {
            Ordering::Less => {
                match self.left.as_mut() {
                    Some(left) => AVLNode::insert(left.as_mut(), value)?,
                    None => self.create_child(value, Orientation::Left),
                }
                self.update_height();
            }
            Ordering::Greater => {
                match self.right.as_mut() {
                    Some(right) => AVLNode::insert(right.as_mut(), value)?,
                    None => self.create_child(value, Orientation::Right),
                }
                self.update_height();
            }
            Ordering::Equal => return Err(Error::InsertionError),
        }

        match self.balance() {
            Balance::LeftHeavy => self.rotate(Orientation::Right),
            Balance::RightHeavy => self.rotate(Orientation::Left),
            Balance::Balanced => Ok(()),
        }
    }

    fn rotate(&mut self, direction: Orientation) -> Result<(), Error> {
        match direction {
            Orientation::Left => match self.right.take() {
                Some(mut right) => {
                    if let Some(mut r_left) = right.left.take() {
                        std::mem::swap(right.as_mut(), r_left.as_mut());
                        right.right = Some(r_left);
                    }
                    std::mem::swap(self, right.as_mut());
                    right.update_height();
                    self.left = Some(right);
                }
                None => unreachable!(),
            },
            Orientation::Right => match self.left.take() {
                Some(mut left) => {
                    if let Some(mut l_right) = left.right.take() {
                        std::mem::swap(left.as_mut(), l_right.as_mut());
                        left.left = Some(l_right);
                    }
                    std::mem::swap(self, left.as_mut());
                    left.update_height();
                    self.right = Some(left);
                }
                None => unreachable!(),
            },
        }
        Ok(())
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum Balance {
    LeftHeavy,
    Balanced,
    RightHeavy,
}

#[cfg(test)]
mod test {
    use super::*;

    fn insert_node<T: Ord + Debug>(tree: &mut AVLTree<T>, value: T) {
        tree.insert(value).expect("unable to insert node");
    }

    #[test]
    fn insert_nodes() {
        let mut tree = AVLTree::new();
        assert_eq!(0, tree.size());

        insert_node(&mut tree, 1);
        assert_eq!(1, tree.size());

        insert_node(&mut tree, 2);
        assert_eq!(2, tree.size());
    }

    #[test]
    fn contains() {
        let mut tree = AVLTree::new();
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
        let mut tree = AVLTree::new();

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
    fn balance() {
        let mut tree = AVLTree::new();

        // Assert tree remains balanced after insertions
        let insertions = [
            3, 4, 5, // Right Heavy
            2, 1, // Left Heavy
            7, 6, // Right Heavy (Left-Right rotation)
        ];

        for i in insertions {
            insert_node(&mut tree, i);
            assert!(tree.is_balanced());
        }

        let nodes: Vec<&u32> = tree.level_iter().collect();
        assert_eq!(vec![&4, &2, &6, &1, &3, &5, &7], nodes);
    }

    #[test]
    fn rotate() {
        let mut tree = AVLTree::new();

        insert_node(&mut tree, 1);
        insert_node(&mut tree, 2);
        insert_node(&mut tree, 3);

        assert_eq!(3, tree.size());
        assert_eq!(1, tree.height());

        let mut level_iter = tree.level_iter();
        assert_eq!(Some(&2), level_iter.next());
        assert_eq!(Some(&1), level_iter.next());
        assert_eq!(Some(&3), level_iter.next());
        assert_eq!(None, level_iter.next());
    }
}
