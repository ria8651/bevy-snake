#![allow(dead_code)]

use bevy::prelude::*;
use rand::{rngs::StdRng, seq::IteratorRandom, SeedableRng};
use serde::{Deserialize, Serialize};
use std::ops::{Index, IndexMut};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Serialize)]
pub enum Cell {
    Empty,
    Wall,
    Snake { id: u8, part: u8 },
    Apple { natural: bool }, // natural apples respawn
}

#[derive(Resource, Component, Clone, Serialize)]
pub struct Board {
    cells: Vec<Cell>,
    #[serde(skip)]
    rng: StdRng,
    width: usize,
    height: usize,
    apples_eaten: usize,
}

impl Board {
    pub fn empty(width: usize, height: usize) -> Self {
        let cells = vec![Cell::Empty; width * height];
        let rng = StdRng::from_entropy();
        let apples_eaten = 0;
        Self {
            cells,
            rng,
            width,
            height,
            apples_eaten,
        }
    }

    pub fn new(board_settings: BoardSettings) -> Self {
        let (width, height) = match board_settings.board_size {
            BoardSize::Small => (10, 9),
            BoardSize::Medium => (17, 15),
            BoardSize::Large => (24, 21),
        };

        let snakes = board_settings.players as usize;
        let mut board = Self::empty(width, height);

        // add snakes
        if let PlayerCount::One = board_settings.players {
            let offset = match board_settings.board_size {
                BoardSize::Small => 0,
                BoardSize::Medium => 1,
                BoardSize::Large => 3,
            };

            let y = height as i32 / 2;
            for i in 0..4 {
                board[IVec2::new(offset + i, y)] = Cell::Snake {
                    id: 0,
                    part: i as u8,
                };
            }
        } else {
            let positions = vec![
                [(1, -1), (2, -1), (3, -1), (4, -1)],
                [(-1, 1), (-2, 1), (-3, 1), (-4, 1)],
                [(1, 1), (1, 2), (1, 3), (1, 4)],
                [(-1, -1), (-1, -2), (-1, -3), (-1, -4)],
            ];
            for (snake_id, positions) in positions[..snakes].into_iter().enumerate() {
                for (i, (mut x, mut y)) in positions.iter().enumerate() {
                    if x < 0 {
                        x += width as i32 - 1;
                    }
                    if y < 0 {
                        y += height as i32 - 1;
                    }
                    board[IVec2::new(x, y)] = Cell::Snake {
                        id: snake_id as u8,
                        part: i as u8,
                    };
                }
            }
        }

        // add apples
        let apple_center = if let PlayerCount::One = board_settings.players {
            match board_settings.board_size {
                BoardSize::Small => 6,
                BoardSize::Medium => 11,
                BoardSize::Large => 16,
            }
        } else {
            width as i32 / 2
        };
        let apple_pattern = match board_settings.apples {
            AppleCount::One => vec![(1, 0)],
            AppleCount::Three => vec![(1, 0), (-1, 2), (-1, -2)],
            AppleCount::Five => vec![(0, 0), (2, 2), (2, -2), (-2, 2), (-2, -2)],
        };
        let apple_y = height as i32 / 2;
        for (x, y) in apple_pattern {
            board[IVec2::new(apple_center + x, apple_y + y)] = Cell::Apple { natural: true };
        }

        board
    }

    pub fn get(&self, pos: IVec2) -> Result<Cell, CellError> {
        if !self.in_bounds(pos) {
            return Err(CellError::OutOfBounds);
        }
        Ok(self[pos])
    }

    pub fn set(&mut self, pos: IVec2, cell: Cell) -> Result<(), CellError> {
        if !self.in_bounds(pos) {
            return Err(CellError::OutOfBounds);
        }
        self[pos] = cell;
        Ok(())
    }

    pub fn in_bounds(&self, pos: IVec2) -> bool {
        pos.x >= 0 && pos.y >= 0 && pos.x < self.width as i32 && pos.y < self.height as i32
    }

    pub fn spawn_apple(&mut self) -> Result<(), ()> {
        let empty = self
            .cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| matches!(cell, Cell::Empty))
            .choose(&mut self.rng);

        if let Some((i, _)) = empty {
            self.cells[i] = Cell::Apple { natural: true };
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn get_spawnable(&self) -> Vec<IVec2> {
        let w = self.width as i32;
        let h = self.height as i32;

        let unspawnable = vec![
            IVec2::new(1, 0),
            IVec2::new(0, 1),
            IVec2::new(w - 2, 0),
            IVec2::new(0, h - 2),
            IVec2::new(w - 2, h - 1),
            IVec2::new(w - 1, h - 2),
            IVec2::new(1, h - 1),
            IVec2::new(w - 1, 1),
        ];
        let corner_cases = vec![
            (IVec2::new(0, 2), IVec2::new(2, 0)),
            (IVec2::new(0, h - 3), IVec2::new(2, h - 1)),
            (IVec2::new(w - 3, h - 1), IVec2::new(w - 1, h - 3)),
            (IVec2::new(w - 1, 2), IVec2::new(w - 3, 0)),
        ];
        let neighbors = vec![
            IVec2::new(0, 1),
            IVec2::new(1, 1),
            IVec2::new(1, 0),
            IVec2::new(1, -1),
            IVec2::new(0, -1),
            IVec2::new(-1, -1),
            IVec2::new(-1, 0),
            IVec2::new(-1, 1),
        ];

        let snakes = self.snakes().into_iter();
        let heads: Vec<_> = snakes
            .filter_map(|snake| snake.last().map(|(pos, _)| *pos))
            .collect();

        let mut spawnable = Vec::new();
        'outer: for (pos, cell) in self.cells() {
            // only spawn in empty cells
            if !matches!(cell, Cell::Empty) {
                continue;
            }

            // dont spawn in corners
            if unspawnable.contains(&pos) {
                continue;
            }

            // dont spawn next to heads
            for head in heads.iter() {
                if (*head - pos).abs().length_squared() < 9 {
                    continue 'outer;
                }
            }

            // dont spawn next to walls
            for neighbor in neighbors.iter() {
                let neighbor_pos = pos + *neighbor;
                if !self.in_bounds(neighbor_pos) {
                    continue;
                }

                if matches!(self[neighbor_pos], Cell::Wall) {
                    continue 'outer;
                }
            }

            // dont spawn next to each other on walls
            if pos.x == 0 || pos.x == self.width as i32 - 1 {
                for dir in [-1, 1] {
                    let block_pos = IVec2::new(pos.x, pos.y + 2 * dir);
                    if self.in_bounds(block_pos) {
                        if matches!(self[block_pos], Cell::Wall) {
                            continue 'outer;
                        }
                    }
                }
            }
            if pos.y == 0 || pos.y == self.height as i32 - 1 {
                for dir in [-1, 1] {
                    let block_pos = IVec2::new(pos.x + 2 * dir, pos.y);
                    if self.in_bounds(block_pos) {
                        if matches!(self[block_pos], Cell::Wall) {
                            continue 'outer;
                        }
                    }
                }
            }

            // stop from trapping snake in corner
            for (corner, corner_opposite) in corner_cases.iter() {
                if pos == *corner || pos == *corner_opposite {
                    if matches!(self[*corner], Cell::Wall) {
                        continue 'outer;
                    }
                    if matches!(self[*corner_opposite], Cell::Wall) {
                        continue 'outer;
                    }
                }
            }

            spawnable.push(pos);
        }

        spawnable
    }

    pub fn spawn_wall(&mut self) -> Result<(), ()> {
        let spawnable = self.get_spawnable();
        let pos = spawnable.into_iter().choose(&mut self.rng).ok_or(())?;
        self[pos] = Cell::Wall;
        Ok(())
    }

    fn damage_snake(&mut self, snake_id: u8) -> Result<(), ()> {
        let snakes = self.snakes();
        for (pos, _) in snakes.get(snake_id as usize).ok_or(())? {
            self[*pos] = Cell::Apple { natural: false };
        }
        Ok(())
    }

    pub fn tick_board(&mut self, inputs: &[Option<Direction>]) -> Result<(), BoardError> {
        let snakes = self.snakes();
        let snake_count = snakes.len();

        let mut grow = vec![false; snake_count];
        let mut damage = vec![false; snake_count];
        let mut spawn_apples = 0;
        let mut new_heads = Vec::new();
        for (snake_id, snake) in snakes.into_iter().enumerate() {
            if snake.is_empty() {
                continue;
            }

            if snake.len() < 2 {
                return Err(BoardError::SnakeTooShort {
                    snake: snake_id as u8,
                });
            }

            let (head, head_part) = snake[snake.len() - 1];
            let (neck, _) = snake[snake.len() - 2];

            let input = *inputs.get(snake_id).ok_or(BoardError::NotEnoughInputs)?;
            let current_dir = Direction::try_from(head - neck).map_err(|_| {
                BoardError::HeadNotAttachedToNeck {
                    snake: snake_id as u8,
                }
            })?;

            // dont allow going back
            let dir = match input {
                Some(d) => {
                    if d != current_dir.opposite() {
                        d
                    } else {
                        current_dir
                    }
                }
                None => current_dir,
            };

            let new_head = head + dir.as_vec2();
            if !self.in_bounds(new_head) {
                damage[snake_id] = true;
                continue;
            }

            match self[new_head] {
                Cell::Apple { natural } => {
                    grow[snake_id] = true;
                    if natural {
                        spawn_apples += 1;
                    }
                }
                Cell::Wall => {
                    damage[snake_id] = true;
                    continue;
                }
                Cell::Snake { id, part } => {
                    if id != snake_id as u8 || part != 0 {
                        damage[snake_id] = true;
                        continue;
                    }
                }
                _ => {}
            }

            self[new_head] = Cell::Snake {
                id: snake_id as u8,
                part: head_part + 1,
            };

            new_heads.push((new_head, head_part));
        }

        for i in 0..snake_count {
            if damage[i] {
                self.damage_snake(i as u8).unwrap();
            }
        }

        for (_, cell) in self.cells_mut() {
            if let Cell::Snake { id, part } = cell {
                if !grow[*id as usize] {
                    if *part == 0 {
                        *cell = Cell::Empty;
                    } else {
                        *cell = Cell::Snake {
                            id: *id,
                            part: *part - 1,
                        };
                    }
                }
            }
        }

        for _ in 0..spawn_apples {
            self.spawn_apple().ok();
            self.apples_eaten += 1;

            if self.apples_eaten % 2 == 1 {
                self.spawn_wall().ok();
            }
        }

        Ok(())
    }

    pub fn cells(&self) -> impl Iterator<Item = (IVec2, Cell)> + '_ {
        self.cells.iter().enumerate().map(|(i, cell)| {
            let x = i % self.width;
            let y = i / self.width;
            (IVec2::new(x as i32, y as i32), *cell)
        })
    }

    pub fn cells_mut(&mut self) -> impl Iterator<Item = (IVec2, &mut Cell)> + '_ {
        self.cells.iter_mut().enumerate().map(|(i, cell)| {
            let x = i % self.width;
            let y = i / self.width;
            (IVec2::new(x as i32, y as i32), cell)
        })
    }

    pub fn snakes(&self) -> Vec<Vec<(IVec2, u8)>> {
        let mut snakes = Vec::new();
        for (pos, cell) in self.cells() {
            if let Cell::Snake { id, part } = cell {
                while snakes.len() <= id as usize {
                    snakes.push(Vec::new());
                }
                snakes[id as usize].push((pos, part));
            }
        }
        for i in 0..snakes.len() {
            snakes[i].sort_unstable_by_key(|(_, part)| *part);
        }
        snakes
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// Returns the number of living snakes. The snake ids can be larger than the number of players.
    pub fn count_snakes(&self) -> usize {
        let mut snake_ids = Vec::new();
        for (_, cell) in self.cells() {
            if let Cell::Snake { id, .. } = cell {
                snake_ids.push(id);
            }
        }
        snake_ids.sort_unstable();
        snake_ids.dedup();
        snake_ids.len()
    }
}

impl Index<IVec2> for Board {
    type Output = Cell;

    fn index(&self, index: IVec2) -> &Self::Output {
        if !self.in_bounds(index) {
            panic!("Index out of bounds");
        }
        &self.cells[index.y as usize * self.width + index.x as usize]
    }
}

impl IndexMut<IVec2> for Board {
    fn index_mut(&mut self, index: IVec2) -> &mut Self::Output {
        if !self.in_bounds(index) {
            panic!("Index out of bounds");
        }
        &mut self.cells[index.y as usize * self.width + index.x as usize]
    }
}

impl std::fmt::Debug for Board {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for y in 0..self.height {
            for x in 0..self.width {
                let cell = self[IVec2::new(x as i32, y as i32)];
                let c = match cell {
                    Cell::Empty => ' ',
                    Cell::Wall => '#',
                    Cell::Snake { id, .. } => match id {
                        0 => 'A',
                        1 => 'B',
                        2 => 'C',
                        3 => 'D',
                        _ => unreachable!(),
                    },
                    Cell::Apple { .. } => 'o',
                };
                write!(f, "{}", c)?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

#[derive(Reflect, PartialEq, Eq, Clone, Copy, Debug)]
pub enum BoardSize {
    Small,
    Medium,
    Large,
}

#[derive(Reflect, PartialEq, Eq, Clone, Copy, Debug)]
pub enum AppleCount {
    One = 1,
    Three = 3,
    Five = 5,
}

#[derive(Reflect, PartialEq, Eq, Clone, Copy, Debug)]
pub enum PlayerCount {
    One = 1,
    Two = 2,
    Three = 3,
    Four = 4,
}

#[derive(Reflect, Clone, Copy, Debug)]
pub struct BoardSettings {
    pub board_size: BoardSize,
    pub apples: AppleCount,
    pub players: PlayerCount,
}

impl Default for BoardSettings {
    fn default() -> Self {
        Self {
            board_size: BoardSize::Small,
            apples: AppleCount::Five,
            players: PlayerCount::One,
        }
    }
}

#[derive(Error, Debug)]
pub enum BoardError {
    #[error("Not enough inputs")]
    NotEnoughInputs,
    #[error("Snake {snake} has less than 2 parts")]
    SnakeTooShort { snake: u8 },
    #[error("Head not attached to neck for snake {snake}")]
    HeadNotAttachedToNeck { snake: u8 },
}

#[derive(Error, Debug)]
pub enum CellError {
    #[error("Cell lookup out of bounds")]
    OutOfBounds,
}

#[derive(PartialEq, Clone, Copy, Deserialize, Serialize, Debug)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    pub const ALL: [Direction; 4] = [
        Direction::Up,
        Direction::Down,
        Direction::Left,
        Direction::Right,
    ];

    pub const DIR: [[i32; 2]; 4] = [[0, 1], [0, -1], [-1, 0], [1, 0]];

    pub fn opposite(&self) -> Self {
        match self {
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
        }
    }

    pub fn as_vec2(&self) -> IVec2 {
        (*self).into()
    }
}

impl From<Direction> for IVec2 {
    fn from(dir: Direction) -> Self {
        match dir {
            Direction::Up => IVec2::new(0, 1),
            Direction::Down => IVec2::new(0, -1),
            Direction::Left => IVec2::new(-1, 0),
            Direction::Right => IVec2::new(1, 0),
        }
    }
}

impl TryFrom<IVec2> for Direction {
    type Error = ();

    fn try_from(value: IVec2) -> Result<Self, Self::Error> {
        match value.to_array() {
            [0, 1] => Ok(Direction::Up),
            [0, -1] => Ok(Direction::Down),
            [1, 0] => Ok(Direction::Right),
            [-1, 0] => Ok(Direction::Left),
            _ => Err(()),
        }
    }
}

impl TryFrom<usize> for Direction {
    type Error = ();

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Direction::Up),
            1 => Ok(Direction::Right),
            2 => Ok(Direction::Down),
            3 => Ok(Direction::Left),
            _ => Err(()),
        }
    }
}
