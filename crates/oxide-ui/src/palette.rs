//! The two visual directions, as concrete colours.
//!
//! Both are shipped: the toggle in the header switches the whole workbench,
//! panels and 3D view alike. Domain colours (good/warn/bad) are the same
//! semantic slots in both, so a red tool light stays red.

use nsh_core::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub background: u32,
    pub surface: u32,
    pub surface_alt: u32,
    pub border: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub accent_text: u32,
    pub good: u32,
    pub warn: u32,
    pub bad: u32,
    pub info: u32,
    /// The 3D clear colour.
    pub scene: u32,
    pub scene_grid: u32,
    pub scene_accent: u32,
}

/// "Porcelain & Ink": a bright control room, hairline borders, one teal accent.
pub const LIGHT: Palette = Palette {
    background: 0xf2f5f8,
    surface: 0xffffff,
    surface_alt: 0xe9eef5,
    border: 0xd5dee9,
    text: 0x101828,
    muted: 0x5b6472,
    accent: 0x0f766e,
    accent_text: 0xffffff,
    good: 0x15803d,
    warn: 0xb45309,
    bad: 0xb91c1c,
    info: 0x1d4ed8,
    scene: 0xe8eef5,
    scene_grid: 0xc3cfdd,
    scene_accent: 0xb45309,
};

/// "Cleanroom Night": the default; deep navy, luminous tool states.
pub const DARK: Palette = Palette {
    background: 0x070b11,
    surface: 0x0e1620,
    surface_alt: 0x151f2b,
    border: 0x22303f,
    text: 0xe6edf5,
    muted: 0x8b9bb0,
    accent: 0x2dd4bf,
    accent_text: 0x04211e,
    good: 0x34d399,
    warn: 0xfbbf24,
    bad: 0xf87171,
    info: 0x60a5fa,
    scene: 0x070c13,
    scene_grid: 0x1b2634,
    scene_accent: 0xfbbf24,
};

impl Palette {
    #[must_use]
    pub fn color(value: u32) -> AdaptiveColor {
        AdaptiveColor::from(rgb(value))
    }

    #[must_use]
    pub fn scene_floats(value: u32) -> [f32; 3] {
        [
            ((value >> 16) & 0xff) as f32 / 255.0,
            ((value >> 8) & 0xff) as f32 / 255.0,
            (value & 0xff) as f32 / 255.0,
        ]
    }

    /// The scene theme for this palette.
    #[must_use]
    pub fn scene_theme(self) -> oxide_scene::SceneTheme {
        let [r, g, b] = Self::scene_floats(self.scene);
        let [gr, gg, gb] = Self::scene_floats(self.scene_grid);
        oxide_scene::SceneTheme {
            background: [r, g, b, 1.0],
            accent: Self::scene_floats(self.scene_accent),
            dim_factor: if self == LIGHT { 0.42 } else { 0.34 },
            grid: [gr, gg, gb, 0.55],
        }
    }
}
