//! Replace Bevy's bundled default font (FiraMono-subset, missing geometric
//! glyphs like `●` and `○`) with the fuller Fira Mono Medium already in
//! `assets/fonts/`.
//!
//! Bevy registers its embedded font at `AssetId::default()` in
//! [`bevy_text::TextPlugin::build`]; any `TextFont` whose `font` field is
//! `Handle::default()` resolves to that slot. We overwrite the slot once,
//! at app build time, *after* `DefaultPlugins` has populated it — so every
//! existing `TextFont::default()` automatically picks up our font with
//! zero per-entity bookkeeping.
//!
//! The TTF is `include_bytes!`'d so the asset is available synchronously
//! during plugin build. Avoids the timing gap of `asset_server.load(...)`
//! (handle exists immediately but the font data isn't decoded yet, so any
//! text spawned in `Startup` would render before our font loads).

use bevy::asset::{AssetId, Assets};
use bevy::prelude::*;
use bevy::text::Font;

const FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/FiraMono-Medium.ttf");

pub struct DefaultFontPlugin;

impl Plugin for DefaultFontPlugin {
    fn build(&self, app: &mut App) {
        let font = Font::try_from_bytes(FONT_BYTES.to_vec())
            .expect("FiraMono-Medium.ttf parses as a Font asset");
        let mut fonts = app.world_mut().resource_mut::<Assets<Font>>();
        fonts
            .insert(AssetId::default(), font)
            .expect("overwrite default font asset");
    }
}
