//! Colours, fonts and layout constants.

use bevy::prelude::*;
use bevy::text::{FontSize, FontSource, FontWeight};

pub const FONT_FAMILY: &str = "Segoe UI";

pub fn font(px: f32, weight: FontWeight) -> TextFont {
    TextFont {
        font: FontSource::Family(FONT_FAMILY.into()),
        font_size: FontSize::Px(px),
        weight,
        ..default()
    }
}

/// Windows 11's icon font (falls back to MDL2 Assets on Windows 10).
pub fn icon_font(px: f32) -> TextFont {
    let family = if std::path::Path::new(r"C:\Windows\Fonts\SegoeIcons.ttf").exists() {
        "Segoe Fluent Icons"
    } else {
        "Segoe MDL2 Assets"
    };
    TextFont { font: FontSource::Family(family.into()), font_size: FontSize::Px(px), ..default() }
}

pub const WHITE: Color = Color::srgb(0.97, 0.98, 1.0);

/// Category accent colours (used only for underline, glow and the Play button glow).
pub const ALL: Color = Color::srgb(0.949, 0.788, 0.298); // #f2c94c
pub const RECENT: Color = Color::srgb(0.184, 0.816, 0.753); // #2fd0c0
pub const STEAM: Color = Color::srgb(0.302, 0.608, 1.0); // #4d9bff
pub const PC: Color = Color::srgb(1.0, 0.604, 0.235); // #ff9a3c
pub const XBOX: Color = Color::srgb(0.435, 0.863, 0.227); // #6fdc3a
pub const XBOX360: Color = Color::srgb(0.839, 0.357, 0.816); // #d65bd0
pub const WISHLIST: Color = Color::srgb(1.0, 0.510, 0.639); // #ff82a3
pub const CONTROLLER: Color = Color::srgb(0.561, 0.482, 1.0); // #8f7bff
pub const SYSTEM: Color = Color::srgb(0.655, 0.706, 0.769); // #a7b4c4

/// Face-button colours: (light, dark) for the glossy gradient.
pub const BTN_A: (Color, Color) = (Color::srgb(0.608, 0.890, 0.416), Color::srgb(0.239, 0.604, 0.071));
pub const BTN_B: (Color, Color) = (Color::srgb(1.0, 0.541, 0.478), Color::srgb(0.722, 0.137, 0.102));
pub const BTN_Y: (Color, Color) = (Color::srgb(1.0, 0.886, 0.478), Color::srgb(0.780, 0.604, 0.027));

// Layout, in canvas pixels (the canvas is 900 tall; `UiScale` maps it to the window).
pub const CANVAS_H: f32 = 900.0;
pub const SIDE: f32 = 72.0;
pub const SHELF_BOTTOM: f32 = 70.0;
pub const TILE_W: f32 = 170.0;
pub const TILE_H: f32 = 255.0;
pub const TILE_SEL_W: f32 = 210.0;
pub const TILE_SEL_H: f32 = 315.0;
pub const TILE_GAP: f32 = 22.0;
/// Corner radius of shelf cards.
pub const CARD_R: f32 = 12.0;
/// Floor reflection under the shelf.
pub const REFL_GAP: f32 = 6.0;
pub const REFL_H: f32 = 40.0;
/// The All grid: card size, gap, and where it sits (canvas pixels).
pub const GRID_W: f32 = 100.0;
pub const GRID_H: f32 = 150.0;
pub const GRID_GAP: f32 = 14.0;
pub const GRID_TOP: f32 = 178.0;
pub const GRID_BOTTOM: f32 = 64.0;
/// The selected tile stays this many tiles from the left edge while scrolling.
pub const SHELF_LEAD: usize = 2;

pub fn darken(c: Color, amount: f32) -> Color {
    let s = c.to_srgba();
    Color::srgba(s.red * (1.0 - amount), s.green * (1.0 - amount), s.blue * (1.0 - amount), s.alpha)
}

pub fn alpha(c: Color, a: f32) -> Color {
    c.with_alpha(a)
}
