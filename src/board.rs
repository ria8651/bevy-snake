#![allow(dead_code)]

use bevy::prelude::*;
use rand::Rng;
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
    width: usize,
    height: usize,
    snakes: usize,
}

impl Board {
    pub fn new(width: usize, height: usize, snakes: usize) -> Self {
        let cells = vec![Cell::Empty; width * height];
        Self {
            cells,
            width,
            height,
            snakes,
        }
    }

    pub fn small(snakes: usize) -> Self {
        let mut board = Self::new(10, 9, snakes);

        if snakes == 1 {
            board.set(7, 4, Cell::Apple);
            for i in 0..4 {
                let cell = Cell::Snake {
                    id: 0,
                    part: i as u8,
                };
                board.set(i, 4, cell);
            }
        } else {
            unimplemented!("Only 1 snake is supported for now");
        }

        board
    }

    pub fn get(&self, x: usize, y: usize) -> Cell {
        #[cfg(debug_assertions)]
        {
            if x >= self.width || y >= self.height {
                panic!("Out of bounds: ({}, {})", x, y);
            }
        }
        self.cells[y * self.width + x]
    }

    pub fn set(&mut self, x: usize, y: usize, cell: Cell) {
        #[cfg(debug_assertions)]
        {
            if x >= self.width || y >= self.height {
                panic!("Out of bounds: ({}, {})", x, y);
            }
        }
        self.cells[y * self.width + x] = cell;
    }

    pub fn get_vec(&self, pos: IVec2) -> Cell {
        self.get(pos.x as usize, pos.y as usize)
    }

    pub fn set_vec(&mut self, pos: IVec2, cell: Cell) {
        self.set(pos.x as usize, pos.y as usize, cell);
    }

    pub fn spawn_apple<R: Rng>(&mut self, rng: &mut R) -> Result<(), ()> {
        let empty = self
            .cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| matches!(cell, Cell::Empty))
            .collect::<Vec<_>>();

        if empty.is_empty() {
            return Err(());
        }

        let (i, _) = empty[rng.gen_range(0..empty.len())];
        self.cells[i] = Cell::Apple;

        Ok(())
    }

    pub fn tick_board(&mut self, inputs: &[Option<Direction>]) -> Result<(), BoardError> {
        let mut heads = vec![(IVec2::ZERO, IVec2::ZERO, 0); self.snakes];
        for (pos, cell) in self.cells() {
            if let Cell::Snake { id, part } = cell {
                if heads[id as usize].2 < part {
                    let neck = heads[id as usize].0;
                    heads[id as usize] = (pos, neck, part);
                }
            }
        }

        let mut grow = vec![false; self.snakes];
        for snake in 0..self.snakes {
            let (head, neck, length) = heads[snake];
            let dir = *inputs.get(snake).ok_or(BoardError::NotEnoughInputs)?;

            let current_dir = Direction::try_from(head - neck)
                .map_err(|_| BoardError::HeadNotAttachedToNeck { snake: snake as u8 })?;

            let dir = match dir {
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

            let cell = self.get_vec(new_head);
            match cell {
                Cell::Wall | Cell::Snake { .. } => {
                    return Err(BoardError::GameOver);
                }
                Cell::Apple => {
                    grow[snake] = true;
                }
                Cell::Empty => {}
            }

            self.set_vec(
                new_head,
                Cell::Snake {
                    id: snake as u8,
                    part: length + 1,
                },
            );
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

        info!("ticked board");

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

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn snakes(&self) -> usize {
        self.snakes
    }
}

impl std::fmt::Debug for Board {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for y in 0..self.height {
            for x in 0..self.width {
                let cell = self.get(x, y);
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

#[derive(Error, Debug)]
pub enum BoardError {
    #[error("Not enough inputs")]
    NotEnoughInputs,
    #[error("Head not attached to neck for snake {snake}")]
    HeadNotAttachedToNeck { snake: u8 },
    #[error("Game over")]
    GameOver,
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
