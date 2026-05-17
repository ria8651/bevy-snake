use crate::board::BoardSettings;
use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Speed {
    Slow,
    Normal,
    Fast,
}

impl Speed {
    pub fn frames_per_movement(self) -> u32 {
        match self {
            Speed::Slow => 12,
            Speed::Normal => 8,
            Speed::Fast => 5,
        }
    }
}

#[derive(Resource, Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GameSettings {
    pub board: BoardSettings,
    pub speed: Speed,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            board: BoardSettings::default(),
            speed: Speed::Normal,
        }
    }
}
