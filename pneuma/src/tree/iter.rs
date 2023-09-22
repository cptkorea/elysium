use super::BoxedNode;
use std::collections::VecDeque;

#[cfg(test)]
pub(super) struct LevelIterator<'a, T: Ord> {
    pub(super) curr: Option<&'a BoxedNode<T>>,
    pub(super) queue: VecDeque<&'a BoxedNode<T>>,
}

pub struct TreeRefIterator<'a, T: Ord> {
    pub(crate) curr: Option<&'a BoxedNode<T>>,
    pub(crate) queue: VecDeque<&'a BoxedNode<T>>,
}

pub struct TreeIterator<T: Ord> {
    pub(crate) curr: Option<BoxedNode<T>>,
    pub(crate) queue: VecDeque<BoxedNode<T>>,
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

#[cfg(test)]
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
