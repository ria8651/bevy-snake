//! Single-slot "say something to the user" surface for the netcode stack.
//!
//! The lobby + matchbox + GGRS layers all push transient and fatal messages
//! through this resource; [src/ui.rs] reads it and renders a banner. Single
//! slot (no queue) because in practice only one issue is live at a time and
//! a queue would mean users staring at a stale "lobby ws error" while the
//! next one ("WebRTC failed") is what they actually need to see.

use bevy::prelude::*;

#[derive(Resource, Default)]
pub struct Notice(pub Option<NoticeEntry>);

#[derive(Clone, Debug)]
pub struct NoticeEntry {
    pub level: NoticeLevel,
    pub message: String,
    /// `Time<Real>::elapsed_secs_f64` at creation, used to drive auto-dismiss.
    pub created: f64,
    /// `None` = sticky until the user dismisses or it's overwritten.
    pub auto_dismiss_secs: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLevel {
    Warn,
    Error,
}

impl Notice {
    pub fn error(&mut self, time: &Time<Real>, msg: impl Into<String>) {
        self.set(NoticeLevel::Error, msg.into(), None, time);
    }

    pub fn warn(&mut self, time: &Time<Real>, msg: impl Into<String>) {
        self.set(NoticeLevel::Warn, msg.into(), Some(6.0), time);
    }

    pub fn clear(&mut self) {
        self.0 = None;
    }

    /// Clear only if the current notice is non-fatal. Fatal notices stick
    /// until acknowledged. Used by recovery paths (e.g. lobby WS reopening,
    /// GGRS `NetworkResumed`) so they don't wipe a still-relevant error.
    pub fn clear_transient(&mut self) {
        if matches!(self.0.as_ref().map(|n| n.level), Some(NoticeLevel::Error)) {
            return;
        }
        self.0 = None;
    }

    fn set(
        &mut self,
        level: NoticeLevel,
        message: String,
        auto_dismiss_secs: Option<f32>,
        time: &Time<Real>,
    ) {
        self.0 = Some(NoticeEntry {
            level,
            message,
            created: time.elapsed_secs_f64(),
            auto_dismiss_secs,
        });
    }
}

pub struct NoticePlugin;

impl Plugin for NoticePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Notice>()
            .add_systems(Update, tick_auto_dismiss);
    }
}

fn tick_auto_dismiss(mut notice: ResMut<Notice>, time: Res<Time<Real>>) {
    let Some(entry) = notice.0.as_ref() else {
        return;
    };
    let Some(ttl) = entry.auto_dismiss_secs else {
        return;
    };
    if time.elapsed_secs_f64() - entry.created >= ttl as f64 {
        notice.0 = None;
    }
}
