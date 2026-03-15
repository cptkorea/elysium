//! # Directed Acyclic Graph — Dependency Resolution
//!
//! This module provides a [`DirectedAcyclicGraph`] for modelling and resolving
//! ordered dependencies between string-keyed nodes. The graph maintains
//! forward and reverse adjacency lists and supports:
//!
//! - Root / leaf discovery
//! - Upstream / downstream neighbour queries
//! - Topological sorting via
//!   [Kahn's algorithm](https://en.wikipedia.org/wiki/Topological_sorting#Kahn's_algorithm)
//!   (which doubles as cycle detection)

use std::collections::{HashMap, HashSet, VecDeque};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("cycle detected involving task \"{0}\"")]
    CycleDetected(String),
}

/// A directed acyclic graph representing task dependencies.
///
/// The graph encodes dependencies as two adjacency lists (forward and
/// reverse) plus a node set. It provides query methods for traversal
/// ([`roots`](Self::roots), [`leaves`](Self::leaves),
/// [`dependencies_of`](Self::dependencies_of),
/// [`dependents_of`](Self::dependents_of)) and a topological sort
/// ([`execution_order`](Self::execution_order)) that doubles as cycle
/// detection.
///
/// # Graph Structure
///
/// Node names are interned into a contiguous `Vec<String>` and all
/// internal operations use `usize` indices. This avoids per-edge string
/// cloning and replaces hash-map lookups in hot paths with direct
/// indexing.
///
/// - `names[i]`: the human-readable name of node `i`
/// - `dependencies[i]`: indices of nodes that node `i` depends on
/// - `dependents[i]`: indices of nodes that depend on node `i`
#[derive(Debug)]
pub struct DirectedAcyclicGraph {
    /// index -> node name
    names: Vec<String>,
    /// node name -> index (reverse lookup for public `&str`-based API)
    index_of: HashMap<String, usize>,
    /// node index -> indices of its upstream dependencies
    dependencies: Vec<Vec<usize>>,
    /// node index -> indices of its downstream dependents
    dependents: Vec<Vec<usize>>,
}

impl DirectedAcyclicGraph {
    /// Creates a new graph from a list of node names and directed edges.
    ///
    /// Each edge is a `(dependency, dependent)` pair of indices into
    /// `names`. The caller is responsible for ensuring all indices are
    /// within bounds.
    pub fn new(names: Vec<String>, edges: Vec<(usize, usize)>) -> Self {
        let n = names.len();
        let index_of: HashMap<String, usize> = names
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), i))
            .collect();

        let mut dependencies = vec![Vec::new(); n];
        let mut dependents = vec![Vec::new(); n];

        for (dep, dependent) in edges {
            dependencies[dependent].push(dep);
            dependents[dep].push(dependent);
        }

        Self {
            names,
            index_of,
            dependencies,
            dependents,
        }
    }

    /// Returns nodes with no dependencies (entry points of the graph).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let roots = graph.roots();
    /// // For a graph with edges [a, b] -> [c]:
    /// // roots == ["a", "b"]
    /// ```
    pub fn roots(&self) -> Vec<&str> {
        (0..self.names.len())
            .filter(|&i| self.dependencies[i].is_empty())
            .map(|i| self.names[i].as_str())
            .collect()
    }

    /// Returns nodes with no dependents (terminal nodes of the graph).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let leaves = graph.leaves();
    /// // For a graph [a, b] -> [c] -> [d]:
    /// // leaves == ["d"]
    /// ```
    pub fn leaves(&self) -> Vec<&str> {
        (0..self.names.len())
            .filter(|&i| self.dependents[i].is_empty())
            .map(|i| self.names[i].as_str())
            .collect()
    }

    /// Returns the names of the direct upstream dependencies of a node.
    ///
    /// An empty vec means the node has no dependencies (it is a root).
    /// Returns an empty vec if the node name is not in the graph.
    pub fn dependencies_of(&self, task: &str) -> Vec<&str> {
        self.index_of
            .get(task)
            .map(|&i| {
                self.dependencies[i]
                    .iter()
                    .map(|&j| self.names[j].as_str())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns the names of the direct downstream dependents of a node.
    ///
    /// An empty vec means nothing depends on this node (it is a leaf).
    /// Returns an empty vec if the node name is not in the graph.
    pub fn dependents_of(&self, task: &str) -> Vec<&str> {
        self.index_of
            .get(task)
            .map(|&i| {
                self.dependents[i]
                    .iter()
                    .map(|&j| self.names[j].as_str())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns the total number of nodes in the graph.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Returns `true` if the graph contains no nodes.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Returns nodes in a valid execution order using
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
    pub fn execution_order(&self) -> Result<Vec<String>, Error> {
        let n = self.names.len();
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
            let remaining: Vec<&str> = (0..n)
                .filter(|i| !visited.contains(i))
                .map(|i| self.names[i].as_str())
                .collect();
            return Err(Error::CycleDetected(remaining.join(", ")));
        }

        Ok(order.into_iter().map(|i| self.names[i].clone()).collect())
    }
}
