use std::ptr::NonNull;

type Link = Option<NonNull<Node>>;

pub struct SkipList {
    height: usize,
    head: NonNull<Node>,
}

#[derive(Debug, Default)]
struct Node {
    value: u32,
    height: usize,
    successors: Vec<Option<NonNull<Node>>>,
}

trait NodeOp {
    fn get_value(self) -> u32;
    fn get_successor(self, i: usize) -> Link;
    fn update_successor(self, i: usize, next: Link);
}

impl NodeOp for NonNull<Node> {
    fn get_value(self) -> u32 {
        unsafe { (*self.as_ptr()).value }
    }

    fn get_successor(self, i: usize) -> Link {
        let height = unsafe { (*self.as_ptr()).height };
        if i >= height {
            return None;
        }

        let successors = unsafe { &(*self.as_ptr()).successors };
        successors[i]
    }

    fn update_successor(self, i: usize, next: Link) {
        unsafe { (*self.as_ptr()).successors[i] = next };
    }
}

impl Node {
    fn new(value: u32, height: usize) -> Self {
        let mut successors: Vec<Link> = Vec::with_capacity(height);
        (0..height).for_each(|_| successors.push(None));

        Self {
            value,
            height,
            successors,
        }
    }

    fn create_nonnull(value: u32, height: usize) -> NonNull<Node> {
        let new_node = Node::new(value, height);
        unsafe { NonNull::new_unchecked(Box::into_raw(Box::new(new_node))) }
    }
}

impl SkipList {
    pub fn new(height: usize) -> Self {
        Self {
            height,
            head: Node::create_nonnull(0, height),
        }
    }

    pub fn insert(&mut self, value: u32) {
        let mut predecessors = Vec::with_capacity(self.height);
        let mut curr = self.head;
        for i in (0..self.height).rev() {
            while let Some(next) = curr.get_successor(i) {
                if next.get_value() < value {
                    curr = next;
                }
            }
            predecessors.push(curr.clone());
        }

        let pos = curr.get_successor(0);
        if pos.is_none() || pos.is_some_and(|n| n.get_value() != value) {
            let level = 2;
            let new_node = Node::create_nonnull(value, level);

            for i in 0..level {
                let prev = predecessors[self.height - i - 1];
                new_node.update_successor(i, prev.get_successor(i));
                prev.update_successor(i, Some(new_node));
            }
        }
    }

    pub fn display(&self) -> Vec<String> {
        let mut levels = Vec::with_capacity(self.height);

        for i in 0..self.height {
            let mut values = Vec::new();
            let mut curr = self.head.clone();
            while let Some(next) = curr.get_successor(i) {
                values.push(next.get_value().to_string());
                curr = next.clone();
            }

            let mut level_str = values.iter().fold(String::new(), |mut s, n| {
                s.push_str(n.as_str());
                s.push_str("->");
                s
            });
            level_str.push_str("x");

            levels.push(level_str);
        }

        levels
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn insertion() {
        let mut skiplist = SkipList::new(5);
        skiplist.insert(1);
        skiplist.insert(2);

        let res: Vec<_> = skiplist.display();
        assert_eq!(vec!["1->2->x", "1->2->x", "x", "x", "x"], res);
    }
}
