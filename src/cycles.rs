use crate::board::{Board, Cell, Direction};
use bevy::prelude::*;
use rand::{seq::SliceRandom, Rng};
use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    ops::{BitAnd, BitXor, Deref, Index, IndexMut},
};

#[derive(PartialEq)]
pub struct Graph {
    pub board: Board,
    pub nodes: HashMap<IVec2, usize>,
    pub nodes_reverse: HashMap<usize, IVec2>,
    pub connections: Vec<Vec<usize>>,
    pub edges: Vec<(usize, usize)>,
    pub edge_connections: HashMap<usize, Vec<usize>>,
}

impl Graph {
    pub fn from_board(board: &Board) -> Self {
        // create a graph representation of the board
        let mut nodes = HashMap::new();
        let mut connections = Vec::new();
        for (pos, cell) in board.cells() {
            if !matches!(cell, Cell::Wall) {
                nodes.insert(pos, nodes.len());
                connections.push(Vec::new());
            }
        }
        for (node, index) in nodes.iter() {
            for dir in Direction::ALL {
                let next_node = *node + dir.as_vec2();
                if let Some(next_index) = nodes.get(&next_node) {
                    connections[*index].push(*next_index);
                }
            }
        }

        // create reverse mapping for nodes
        let nodes_reverse: HashMap<usize, IVec2> = nodes.iter().map(|(k, v)| (*v, *k)).collect();

        // make list of edges
        let mut edges = Vec::new();
        for (index, neighbors) in connections.iter().enumerate() {
            for &neighbor in neighbors {
                if neighbor > index {
                    edges.push((index, neighbor));
                }
            }
        }

        // find how edges are connected through vertices
        let mut edge_connections: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, &(a, b)) in edges.iter().enumerate() {
            for (j, &(k, l)) in edges.iter().enumerate() {
                if i != j && (a == k || b == k || a == l || b == l) {
                    edge_connections.entry(i).or_default().push(j);
                }
            }
        }

        Graph {
            board: board.clone(),
            nodes,
            nodes_reverse,
            connections,
            edges,
            edge_connections,
        }
    }

    pub fn edge_index(&self, a: usize, b: usize) -> Option<usize> {
        self.edges
            .iter()
            .position(|&(x, y)| (x == a && y == b) || (x == b && y == a))
    }

    pub fn cycle_basis(&self) -> Vec<EdgeMask> {
        let mut cycles: Vec<Vec<usize>> = Vec::new();
        let root_index = 0;
        // Stack (ie "pushdown list") of vertices already in the spanning tree
        let mut stack: Vec<usize> = vec![root_index];
        // Map of node index to predecessor node index
        let mut pred: HashMap<usize, usize> = HashMap::new();
        pred.insert(root_index, root_index);
        // Set of examined nodes during this iteration
        let mut used: HashMap<usize, HashSet<usize>> = HashMap::new();
        used.insert(root_index, HashSet::new());
        // Walk the spanning tree
        while !stack.is_empty() {
            // Use the last element added so that cycles are easier to find
            let z = stack.pop().unwrap();
            for neighbor in self.connections[z].iter().copied() {
                // A new node was encountered:
                if !used.contains_key(&neighbor) {
                    pred.insert(neighbor, z);
                    stack.push(neighbor);
                    let mut temp_set: HashSet<usize> = HashSet::new();
                    temp_set.insert(z);
                    used.insert(neighbor, temp_set);
                // A self loop:
                } else if z == neighbor {
                    let cycle: Vec<usize> = vec![z];
                    cycles.push(cycle);
                // A cycle was found:
                } else if !used.get(&z).unwrap().contains(&neighbor) {
                    let pn = used.get(&neighbor).unwrap();
                    let mut cycle: Vec<usize> = vec![neighbor, z];
                    let mut p = pred.get(&z).unwrap();
                    while !pn.contains(p) {
                        cycle.push(*p);
                        p = pred.get(p).unwrap();
                    }
                    cycle.push(*p);
                    cycles.push(cycle);
                    let neighbor_set = used.get_mut(&neighbor).unwrap();
                    neighbor_set.insert(z);
                }
            }
        }

        cycles
            .iter()
            .map(|cycle| {
                let mut mask = EdgeMask::new(self);
                for i in 0..cycle.len() {
                    let a = cycle[i];
                    let b = cycle[(i + 1) % cycle.len()];
                    if let Some(index) = self.edge_index(a, b) {
                        mask[index] = true;
                    }
                }
                mask
            })
            .collect()
    }

    pub fn longest_cycle_evolution(
        &self,
        population_size: usize,
        generations: usize,
        mutations: usize,
        rng: &mut impl Rng,
    ) -> EdgeMask {
        let cycles = self.cycle_basis();
        let mut population = Vec::with_capacity(population_size);
        for i in 0..population_size {
            population.push((i, cycles[i % cycles.len()].clone()));
        }
        let half = population.len() / 2;

        for _gen in 0..generations {
            for (_, cycle) in population.iter_mut() {
                for other_cycle in cycles.choose_multiple(rng, mutations) {
                    if cycle.overlap(other_cycle) {
                        let temp = cycle.xor(other_cycle);
                        if temp.valid() {
                            *cycle = temp;
                        }
                    }
                }
            }

            population.sort_by_key(|(_, cycle)| cycle.len());
            population.reverse();

            // println!("Best from gen {}: {}", _gen, population[0].len());
            // println!("{:?}", population[0]);

            for i in 0..half {
                population[half + i] = population[i].clone();
            }

            // let diversity = population.iter().map(|(i, _)| i).collect::<HashSet<_>>();
            // println!("Generation {}: Diversity {}", _gen, diversity.len());
        }

        population.remove(0).1
    }
}

#[derive(Clone, PartialEq)]
pub struct EdgeMask<'a> {
    pub graph: &'a Graph,
    pub mask: Vec<bool>,
}

impl<'a> EdgeMask<'a> {
    pub fn new(graph: &'a Graph) -> Self {
        EdgeMask {
            graph,
            mask: vec![false; graph.edges.len()],
        }
    }

    /// Returns true if the two masks have any overlapping edges
    pub fn overlap(&self, other: &Self) -> bool {
        let mut overlap = false;
        for (i, &value) in other.mask.iter().enumerate() {
            if value && self.mask[i] {
                overlap = true;
                break;
            }
        }
        overlap
    }

    pub fn len(&self) -> usize {
        self.iter().filter(|&&x| x).count()
    }

    pub fn xor(&self, other: &Self) -> Self {
        let mut result = self.clone();
        for (i, &value) in other.mask.iter().enumerate() {
            result.mask[i] ^= value;
        }
        result
    }

    /// cycle is valid if all parts are connected and each edge has exactly two connections
    pub fn valid(&self) -> bool {
        let mut valid = true;
        let mut visited = EdgeMask::new(self.graph);
        let first = self.iter().position(|&x| x);
        let edge_connections = &self.graph.edge_connections;
        if let Some(first) = first {
            let mut stack = vec![first];
            visited[first] = true;
            while let Some(index) = stack.pop() {
                let mut neighbors = 0;
                for &neighbor in edge_connections.get(&index).unwrap_or(&Vec::new()) {
                    if !visited[neighbor] && self[neighbor] {
                        visited[neighbor] = true;
                        stack.push(neighbor);
                    }
                    if self[neighbor] {
                        neighbors += 1;
                    }
                }
                if neighbors != 2 {
                    valid = false;
                    break;
                }
            }
        }
        valid && &visited == self
    }

    #[allow(dead_code)]
    pub fn edges(&self) -> Vec<(IVec2, IVec2)> {
        let mut edges = Vec::new();
        for (i, &value) in self.mask.iter().enumerate() {
            if value {
                let (a, b) = self.graph.edges[i];
                let pos_a = self.graph.nodes_reverse[&a];
                let pos_b = self.graph.nodes_reverse[&b];
                edges.push((pos_a, pos_b));
            }
        }
        edges
    }
}

impl<'a> Deref for EdgeMask<'a> {
    type Target = [bool];

    fn deref(&self) -> &Self::Target {
        &self.mask
    }
}

impl<'a> Index<usize> for EdgeMask<'a> {
    type Output = bool;

    fn index(&self, index: usize) -> &Self::Output {
        &self.mask[index]
    }
}

impl<'a> IndexMut<usize> for EdgeMask<'a> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.mask[index]
    }
}

impl<'a> Debug for EdgeMask<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for y in 0..self.graph.board.height() {
            if y > 0 {
                for x in 0..self.graph.board.width() {
                    if x > 0 {
                        write!(f, " ")?;
                    }
                    let pos = IVec2::new(x as i32, y as i32);
                    if let Some(node) = self.graph.nodes.get(&pos) {
                        let pos_last = IVec2::new(x as i32, y as i32 - 1);
                        if let Some(node_last) = self.graph.nodes.get(&pos_last) {
                            let edge_index = self.graph.edge_index(*node, *node_last);
                            if let Some(index) = edge_index {
                                if self[index] {
                                    write!(f, "|")?;
                                } else {
                                    write!(f, " ")?;
                                }
                            } else {
                                write!(f, " ")?;
                            }
                        } else {
                            write!(f, " ")?;
                        }
                    } else {
                        write!(f, " ")?;
                    }
                }
            }
            writeln!(f)?;
            for x in 0..self.graph.board.width() {
                let pos = IVec2::new(x as i32, y as i32);
                if let Some(node) = self.graph.nodes.get(&pos) {
                    if x > 0 {
                        let pos_last = IVec2::new(x as i32 - 1, y as i32);
                        if let Some(node_last) = self.graph.nodes.get(&pos_last) {
                            let edge_index = self.graph.edge_index(*node, *node_last);
                            if let Some(index) = edge_index {
                                if self[index] {
                                    write!(f, "-")?;
                                } else {
                                    write!(f, " ")?;
                                }
                            } else {
                                write!(f, " ")?;
                            }
                        } else {
                            write!(f, " ")?;
                        }
                    }

                    write!(f, ".")?;
                } else {
                    if x > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "#")?;
                }
            }
            writeln!(f)?;
        }

        Ok(())
    }
}

impl BitAnd for EdgeMask<'_> {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        let mut result = self;
        for (i, &value) in rhs.mask.iter().enumerate() {
            result.mask[i] &= value;
        }
        result
    }
}

impl BitXor for EdgeMask<'_> {
    type Output = Self;

    fn bitxor(self, rhs: Self) -> Self::Output {
        let mut result = self;
        for (i, &value) in rhs.mask.iter().enumerate() {
            result.mask[i] ^= value;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cycle_stability() {
        // first run snake game with ai to get reasonable board
        // let mut board = Board::new(BoardSettings::default());
        // let ai = TreeSearch {
        //     max_depth: 100,
        //     max_time: Duration::from_millis(5),
        // };
        // let mut moves = 0;
        // while let Ok(direction) = ai.chose_move(&board, &mut None) {
        //     board.tick_board(&[Some(direction)]).unwrap();
        //     moves += 1;
        //     if moves > 500 {
        //         break;
        //     }
        // }

        // println!("Board after AI run (len {}):", board.score() + 4);
        // println!("{:?}", board);

        let board = Board::from_str(
            r#"  3333o556
 23#33#5#6
#233355566
224#o55#0#
2#4oo#5512
2 4#4444#3
2#444#1194
2211111#85
#21#11 o76"#,
        )
        .unwrap();

        println!("Board after AI run (len {}):", board.score() + 4);
        println!("{:?}", board);

        let graph = Graph::from_board(&board);
        for _ in 0..10 {
            let cycle = graph.longest_cycle_evolution(500, 100, 5, &mut rand::thread_rng());
            println!("Cycle found (len {}):", cycle.len());
            println!("{:?}", cycle);
        }
    }
}
