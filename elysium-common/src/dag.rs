//! # Directed Acyclic Graph — Dependency Resolution
//!
//! This module provides a [`DirectedAcyclicGraph`] for modelling and resolving
//! ordered dependencies between nodes of any type. The graph maintains
//! forward and reverse adjacency lists and supports:
//!
//! - Root / leaf discovery
//! - Upstream / downstream neighbour queries
//! - Topological sorting via
//!   [Kahn's algorithm](https://en.wikipedia.org/wiki/Topological_sorting#Kahn's_algorithm)
//!   (which doubles as cycle detection)

use std::collections::{HashSet, VecDeque};
use std::fmt;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("cycle detected involving: \"{0}\"")]
    CycleDetected(String),
}

/// A directed acyclic graph over nodes of type `T`.
///
/// Node values are interned into a contiguous `Vec<T>` and all internal
/// operations use `usize` indices. This avoids per-edge cloning and
/// replaces map lookups in hot paths with direct indexing.
///
/// The struct itself imposes **no trait bounds** on `T`. Bounds appear
/// only on the `impl` blocks that need them — for example, `Display` is
/// required only by the cycle-detection error path.
///
/// - `nodes[i]`: the value of node `i`
/// - `dependencies[i]`: indices of nodes that node `i` depends on
/// - `dependents[i]`: indices of nodes that depend on node `i`
#[derive(Debug)]
pub struct DirectedAcyclicGraph<T> {
    /// index -> node value
    nodes: Vec<T>,
    /// node index -> indices of its upstream dependencies
    dependencies: Vec<Vec<usize>>,
    /// node index -> indices of its downstream dependents
    dependents: Vec<Vec<usize>>,
}

// Core methods — no bounds on T.
impl<T> DirectedAcyclicGraph<T> {
    /// Creates a new graph from a list of node values and directed edges.
    ///
    /// Each edge is a `(dependency, dependent)` pair of indices into
    /// `nodes`. The caller is responsible for ensuring all indices are
    /// within bounds.
    pub fn new(nodes: Vec<T>, edges: Vec<(usize, usize)>) -> Self {
        let n = nodes.len();
        let mut dependencies = vec![Vec::new(); n];
        let mut dependents = vec![Vec::new(); n];

        for (dep, dependent) in edges {
            dependencies[dependent].push(dep);
            dependents[dep].push(dependent);
        }

        Self {
            nodes,
            dependencies,
            dependents,
        }
    }

    /// Returns references to nodes with no dependencies (entry points).
    pub fn roots(&self) -> Vec<&T> {
        (0..self.nodes.len())
            .filter(|&i| self.dependencies[i].is_empty())
            .map(|i| &self.nodes[i])
            .collect()
    }

    /// Returns references to nodes with no dependents (terminal nodes).
    pub fn leaves(&self) -> Vec<&T> {
        (0..self.nodes.len())
            .filter(|&i| self.dependents[i].is_empty())
            .map(|i| &self.nodes[i])
            .collect()
    }

    /// Returns references to the direct upstream dependencies of the node
    /// at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.len()`.
    pub fn dependencies_of(&self, index: usize) -> Vec<&T> {
        self.dependencies[index]
            .iter()
            .map(|&j| &self.nodes[j])
            .collect()
    }

    /// Returns references to the direct downstream dependents of the node
    /// at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.len()`.
    pub fn dependents_of(&self, index: usize) -> Vec<&T> {
        self.dependents[index]
            .iter()
            .map(|&j| &self.nodes[j])
            .collect()
    }

    /// Returns the total number of nodes in the graph.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Returns `true` if the graph contains no nodes.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Returns a reference to the node value at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.len()`.
    pub fn node(&self, index: usize) -> &T {
        &self.nodes[index]
    }

    /// Returns a shared reference to the full node slice.
    pub fn nodes(&self) -> &[T] {
        &self.nodes
    }

    /// Consumes the graph and returns the node value table.
    pub fn into_nodes(self) -> Vec<T> {
        self.nodes
    }
}

// Topological sort — requires Display for error formatting.
impl<T: fmt::Display> DirectedAcyclicGraph<T> {
    /// Returns node indices in a valid execution order using
    /// [Kahn's algorithm](https://en.wikipedia.org/wiki/Topological_sorting#Kahn's_algorithm).
    ///
    /// The algorithm maintains an in-degree count for each node. Nodes with
    /// zero in-degree (no unresolved dependencies) are added to a queue.
    /// As each node is dequeued, its downstream dependents have their
    /// in-degree decremented — newly zero-degree nodes join the queue.
    ///
    /// If the resulting order contains fewer nodes than the graph, at least
    /// one cycle exists among the remaining nodes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::CycleDetected`] listing the nodes involved in the
    /// cycle if a valid topological ordering cannot be produced.
    pub fn execution_order_indices(&self) -> Result<Vec<usize>, Error> {
        let n = self.nodes.len();
        let mut in_degree: Vec<usize> = self.dependencies.iter().map(|d| d.len()).collect();

        let mut queue: VecDeque<usize> = in_degree
            .iter()
            .enumerate()
            .filter(|&(_, deg)| *deg == 0)
            .map(|(i, _)| i)
            .collect();

        let mut order = Vec::with_capacity(n);
        while let Some(node) = queue.pop_front() {
            order.push(node);
            for &downstream in &self.dependents[node] {
                in_degree[downstream] -= 1;
                if in_degree[downstream] == 0 {
                    queue.push_back(downstream);
                }
            }
        }

        if order.len() != n {
            let visited: HashSet<usize> = order.iter().copied().collect();
            let remaining: Vec<String> = (0..n)
                .filter(|i| !visited.contains(i))
                .map(|i| self.nodes[i].to_string())
                .collect();
            return Err(Error::CycleDetected(remaining.join(", ")));
        }

        Ok(order)
    }
}

// Convenience method — requires Clone + Display.
impl<T: Clone + fmt::Display> DirectedAcyclicGraph<T> {
    /// Returns node values in a valid execution order.
    ///
    /// Convenience wrapper around [`execution_order_indices`](Self::execution_order_indices)
    /// that maps indices back to cloned node values.
    pub fn execution_order(&self) -> Result<Vec<T>, Error> {
        self.execution_order_indices()
            .map(|indices| indices.into_iter().map(|i| self.nodes[i].clone()).collect())
    }
}
