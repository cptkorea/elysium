use std::cell::RefCell;
use std::rc::Rc;

type NodeRef = Rc<RefCell<Node>>;

pub struct SkipList {
    height: usize,
    head: NodeRef,
}

#[derive(Debug, Default)]
struct Node {
    value: u32,
    height: usize,
    successors: Vec<Option<NodeRef>>,
}

impl Node {
    fn head(height: usize) -> NodeRef {
        let mut successors = Vec::with_capacity(height);
        (0..height).for_each(|_| successors.push(None));

        Rc::new(RefCell::new(Self {
            value: 0,
            height,
            successors,
        }))
    }

    fn new(value: u32, height: usize) -> NodeRef {
        Rc::new(RefCell::new(Self {
            value,
            height,
            successors: Vec::with_capacity(height),
        }))
    }

    fn get_successor(&self, idx: usize) -> Option<&NodeRef> {
        if idx >= self.height {
            return None;
        }
        self.successors[idx].as_ref()
    }

    fn add_successor(&mut self, idx: usize, successor: NodeRef) {
        self.successors[idx] = Some(successor);
    }
}

impl SkipList {
    pub fn new(height: usize) -> Self {
        Self {
            height,
            head: Node::head(height),
        }
    }

    fn create_successors(&self) -> Vec<Option<NodeRef>> {
        let mut successors = Vec::with_capacity(self.height);
        (0..self.height).for_each(|_| successors.push(None));
        successors
    }

    pub fn insert(&mut self, value: u32) {
        let mut predecessors = Vec::with_capacity(self.height);
        let mut curr = self.head.clone();
        for i in (0..self.height).rev() {
            while let Some(next) = curr.clone().borrow().get_successor(i) {
                if next.borrow().value < value {
                    curr = next.clone();
                }
            }
            predecessors.push(curr.clone());
        }

        let pos = curr.clone().borrow().get_successor(0).cloned();
        if pos.is_none() || pos.is_some_and(|n| n.borrow().value != value) {
            let level = 2;
            let new_node = Node::new(value, level);

            for i in 0..level {
                let mut prev = predecessors[self.height - i - 1].borrow_mut();
                new_node
                    .borrow_mut()
                    .successors
                    .push(prev.get_successor(i).cloned());
                prev.add_successor(i, new_node.clone());
            }
        }
    }

    pub fn display(&self) -> Vec<String> {
        let mut levels = Vec::with_capacity(self.height);

        for i in 0..self.height {
            let mut values = Vec::new();
            let mut curr = self.head.clone();
            while let Some(next) = curr.clone().borrow().get_successor(i) {
                values.push(next.borrow().value.to_string());
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
        let mut skiplist = SkipList::new(10);
        skiplist.insert(1);
        skiplist.insert(2);

        println!("{:?}", skiplist.display());
    }
}
