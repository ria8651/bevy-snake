#![allow(dead_code)]

use std::ops::{Index, IndexMut};

use bevy::prelude::*;
use rand::{rngs::StdRng, seq::IteratorRandom, Rng, SeedableRng};
use thiserror::Error;

#[derive(Clone, Copy, Debug)]
pub enum Cell {
    Empty,
    Wall,
    Snake { id: u8, part: u8 },
    Apple,
}

#[derive(Resource, Component, Clone)]
pub struct Board {
    cells: Vec<Cell>,
    rng: StdRng,
    width: usize,
    height: usize,
    snakes: usize,
}

impl Board {
    pub fn empty(width: usize, height: usize, snakes: usize) -> Self {
        let cells = vec![Cell::Empty; width * height];
        let rng = StdRng::from_entropy();
        Self {
            cells,
            rng,
            width,
            height,
            snakes,
        }
    }

    pub fn new(board_settings: BoardSettings) -> Self {
        let (width, height) = match board_settings.board_size {
            BoardSize::Small => (10, 9),
            BoardSize::Medium => (17, 15),
            BoardSize::Large => (24, 21),
        };

        let snakes = match board_settings.players {
            PlayerCount::One => 1,
            PlayerCount::Two => 2,
            PlayerCount::Three => 3,
            PlayerCount::Four => 4,
        };

        let mut board = Self::empty(width, height, snakes);

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
            board[IVec2::new(apple_center + x, apple_y + y)] = Cell::Apple;
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

    pub fn spawn_apple<R: Rng>(&mut self, rng: &mut R) -> Result<(), ()> {
        let empty = self
            .cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| matches!(cell, Cell::Empty))
            .choose(rng);

        if let Some((i, _)) = empty {
            self.cells[i] = Cell::Apple;
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn tick_board(&mut self, inputs: &[Option<Direction>]) -> Result<(), BoardError> {
        let mut grow = vec![false; self.snakes];
        let mut new_heads = Vec::new();
        for (snake_id, snake) in self.snakes().into_iter().enumerate() {
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
                return Err(BoardError::GameOver);
            }

            match self[new_head] {
                Cell::Apple => {
                    grow[snake_id] = true;
                }
                Cell::Wall => {
                    return Err(BoardError::GameOver);
                }
                Cell::Snake { id, part } => {
                    if id != snake_id as u8 || part != 0 {
                        return Err(BoardError::GameOver);
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

        for i in 0..self.snakes {
            if grow[i] {
                self.spawn_apple(&mut rand::thread_rng()).ok();
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
            snakes[i].sort_by_key(|(_, part)| *part);
        }
        snakes
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn snake_count(&self) -> usize {
        self.snakes
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
                    Cell::Apple => 'o',
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
    One,
    Three,
    Five,
}

#[derive(Reflect, PartialEq, Eq, Clone, Copy, Debug)]
pub enum PlayerCount {
    One,
    Two,
    Three,
    Four,
}

#[derive(Reflect, Clone, Copy, Debug)]
pub struct BoardSettings {
    pub board_size: BoardSize,
    pub apples: AppleCount,
    pub players: PlayerCount,
}

#[derive(Error, Debug)]
pub enum BoardError {
    #[error("Not enough inputs")]
    NotEnoughInputs,
    #[error("Snake {snake} has less than 2 parts")]
    SnakeTooShort { snake: u8 },
    #[error("Head not attached to neck for snake {snake}")]
    HeadNotAttachedToNeck { snake: u8 },
    #[error("Game over")]
    GameOver,
}

#[derive(Error, Debug)]
pub enum CellError {
    #[error("Cell lookup out of bounds")]
    OutOfBounds,
}

#[derive(PartialEq, Clone, Copy)]
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
