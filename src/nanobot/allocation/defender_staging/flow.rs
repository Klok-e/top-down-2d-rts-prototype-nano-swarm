//! Lower-bounded deterministic min-cost circulation.

use std::{cmp::Ordering, collections::BinaryHeap};

#[derive(Debug, Clone, Copy)]
struct FlowEdge {
    to: usize,
    reverse: usize,
    capacity: usize,
    cost: f64,
}

#[derive(Debug, Clone, Copy)]
struct QueueEntry {
    cost: f64,
    node: usize,
}

impl PartialEq for QueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.cost.to_bits() == other.cost.to_bits() && self.node == other.node
    }
}

impl Eq for QueueEntry {}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.node.cmp(&self.node))
    }
}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug)]
pub(super) struct BoundedMinCostFlow {
    graph: Vec<Vec<FlowEdge>>,
    balance: Vec<isize>,
    super_source: usize,
    super_sink: usize,
}

impl BoundedMinCostFlow {
    pub(super) fn new(node_count: usize) -> Self {
        Self {
            graph: vec![Vec::new(); node_count + 2],
            balance: vec![0; node_count + 2],
            super_source: node_count,
            super_sink: node_count + 1,
        }
    }

    fn add_residual_edge(&mut self, from: usize, to: usize, capacity: usize, cost: f64) -> usize {
        let forward = self.graph[from].len();
        let reverse = self.graph[to].len();
        self.graph[from].push(FlowEdge {
            to,
            reverse,
            capacity,
            cost,
        });
        self.graph[to].push(FlowEdge {
            to: from,
            reverse: forward,
            capacity: 0,
            cost: -cost,
        });
        forward
    }

    pub(super) fn add_edge(
        &mut self,
        from: usize,
        to: usize,
        lower: usize,
        upper: usize,
        cost: f64,
    ) -> usize {
        assert!(lower <= upper);
        self.balance[from] -= lower as isize;
        self.balance[to] += lower as isize;
        self.add_residual_edge(from, to, upper - lower, cost)
    }

    pub(super) fn solve(&mut self) {
        let mut required = 0;
        for node in 0..self.balance.len() {
            let amount = self.balance[node];
            if amount > 0 {
                self.add_residual_edge(self.super_source, node, amount as usize, 0.0);
                required += amount as usize;
            } else if amount < 0 {
                self.add_residual_edge(node, self.super_sink, (-amount) as usize, 0.0);
            }
        }

        let mut sent = 0;
        let mut potential = vec![0.0_f64; self.graph.len()];
        while sent < required {
            let mut distance = vec![f64::INFINITY; self.graph.len()];
            let mut predecessor = vec![(usize::MAX, usize::MAX); self.graph.len()];
            let mut pending = BinaryHeap::new();
            distance[self.super_source] = 0.0;
            pending.push(QueueEntry {
                cost: 0.0,
                node: self.super_source,
            });

            while let Some(QueueEntry { cost, node }) = pending.pop() {
                if cost.total_cmp(&distance[node]).is_gt() {
                    continue;
                }
                for (edge_index, edge) in self.graph[node].iter().enumerate() {
                    if edge.capacity == 0 {
                        continue;
                    }
                    let reduced_cost = (edge.cost + potential[node] - potential[edge.to]).max(0.0);
                    let candidate = cost + reduced_cost;
                    if candidate.total_cmp(&distance[edge.to]).is_lt() {
                        distance[edge.to] = candidate;
                        predecessor[edge.to] = (node, edge_index);
                        pending.push(QueueEntry {
                            cost: candidate,
                            node: edge.to,
                        });
                    }
                }
            }
            assert!(
                distance[self.super_sink].is_finite(),
                "bounded flow is feasible"
            );
            for node in 0..self.graph.len() {
                if distance[node].is_finite() {
                    potential[node] += distance[node];
                }
            }

            let mut amount = required - sent;
            let mut node = self.super_sink;
            while node != self.super_source {
                let (previous, edge_index) = predecessor[node];
                amount = amount.min(self.graph[previous][edge_index].capacity);
                node = previous;
            }
            node = self.super_sink;
            while node != self.super_source {
                let (previous, edge_index) = predecessor[node];
                let edge = self.graph[previous][edge_index];
                self.graph[previous][edge_index].capacity -= amount;
                self.graph[node][edge.reverse].capacity += amount;
                node = previous;
            }
            sent += amount;
        }
    }

    pub(super) fn edge_is_saturated(&self, from: usize, edge: usize) -> bool {
        self.graph[from][edge].capacity == 0
    }
}
