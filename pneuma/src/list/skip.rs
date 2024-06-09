use std::cell::RefCell;
use std::ops::Index;
use std::ptr::NonNull;
use std::rc::Rc;

pub struct SkipList {
    head: Node,
}

#[derive(Debug, Default)]
struct Node {
    value: u32,
    height: usize,
    successors: Vec<NonNull<Node>>,
}

impl Node {
    fn next(&self, level: usize) -> Option<NonNull<Node>> {
        self.successors.get(level).copied()
    }
}

impl SkipList {
    pub fn new(levels: usize) -> Self {
        Self {
            head: Node {
                value: 0,
                height: 0,
                successors: Vec::with_capacity(levels),
            },
        }
    }

    #[cfg(test)]
    pub fn insert_naive(&mut self, value: u32, height: usize) {
        let new_node = NonNull::new(&mut Node {
            value,
            height,
            successors: Vec::with_capacity(height),
        })
        .expect("error");

        for h in 0..height {
            let mut curr = self.head.successors.get(h);
            match curr {
                Some(curr) => {
                    while !curr.next(h).is_none() && curr.next(h).unwrap().value > value {
                        curr = curr.successors[h].clone();
                    }
                    curr.successors[h] = new_node.clone();
                }
                None => self.head.successors.push(new_node),
            }
        }
    }
}
