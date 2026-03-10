use super::{
    BinaryTreeNode, BoxedNode, Error,
    iter::{ItemIter, ItemRefIter},
};
use std::collections::VecDeque;

#[derive(Default)]
pub struct BinarySearchTree<T: Ord> {
    pub(super) root: Option<BoxedNode<T>>,
    pub(super) size: usize,
}

impl<T: Ord> BinarySearchTree<T> {
    pub fn new() -> Self {
        Self {
            root: None,
            size: 0,
        }
    }

    pub(super) fn size(&self) -> usize {
        self.size
    }

    pub fn insert(&mut self, item: T) -> Result<(), Error> {
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

    pub(super) fn insert_with_fn<F>(&mut self, item: T, f: F) -> Result<(), Error>
    where
        F: Fn(&mut BinaryTreeNode<T>, T) -> Result<(), Error>,
    {
        match self.root.as_mut() {
            None => {
                self.root = Some(BinaryTreeNode::create(item));
            }
            Some(t) => {
                f(t, item)?;
            }
        }
        self.size += 1;
        Ok(())
    }

    pub(super) fn contains(&self, item: &T) -> bool {
        self.root
            .as_ref()
            .map(|r| r.find(&item).is_some())
            .unwrap_or_default()
    }

    pub fn iter(&self) -> ItemRefIter<'_, T> {
        self.into_iter()
    }

    #[cfg(test)]
    pub(super) fn level_order_items(&self) -> Vec<&T> {
        let mut result = Vec::new();
        let mut queue = VecDeque::new();
        if let Some(root) = self.root.as_ref() {
            queue.push_back(root);
        }
        while let Some(node) = queue.pop_front() {
            result.push(&node.item);
            if let Some(left) = node.left.as_ref() {
                queue.push_back(left);
            }
            if let Some(right) = node.right.as_ref() {
                queue.push_back(right);
            }
        }
        result
    }

    #[cfg(test)]
    pub(super) fn height(&self) -> i32 {
        self.root.as_ref().map_or(0, |r| r.height)
    }
}

impl<'a, T: Ord> IntoIterator for &'a BinarySearchTree<T> {
    type Item = &'a T;
    type IntoIter = ItemRefIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        ItemRefIter {
            curr: self.root.as_ref(),
            queue: VecDeque::with_capacity(10),
        }
    }
}

impl<T: Ord> IntoIterator for BinarySearchTree<T> {
    type Item = T;
    type IntoIter = ItemIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        ItemIter {
            curr: self.root,
            queue: VecDeque::with_capacity(10),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn insert_node<T: Ord>(tree: &mut BinarySearchTree<T>, item: T) {
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

        assert_eq!(tree.level_order_items(), vec![&2, &1, &3]);
    }
}
