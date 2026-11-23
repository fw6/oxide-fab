//! The 3D scene's two palettes.
//!
//! The panels draw from `palette.ts` on the TSX side; the scene needs its clear
//! colour, grid and accent in floats, so the two directions live here as the
//! numbers the renderer takes. Domain colours (good/warn/bad) mean the same in
//! both, so a red tool light stays red.

use oxide_scene::SceneTheme;

/// "Porcelain & Ink": a bright control room.
pub const LIGHT: Palette = Palette {
    scene: 0xe8eef5,
    scene_grid: 0xc3cfdd,
    scene_accent: 0xb45309,
    dim_factor: 0.42,
};

/// "Cleanroom Night": the default; deep navy, luminous tool states.
pub const DARK: Palette = Palette {
    scene: 0x070c13,
    scene_grid: 0x1b2634,
    scene_accent: 0xfbbf24,
    dim_factor: 0.34,
};

/// The colours the scene builder uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// The clear colour.
    pub scene: u32,
    /// The floor grid.
    pub scene_grid: u32,
    /// The accent an active level draws with.
    pub scene_accent: u32,
    /// How far the other levels fade back.
    pub dim_factor: f32,
}

impl Palette {
    #[must_use]
    pub fn floats(value: u32) -> [f32; 3] {
        [
            ((value >> 16) & 0xff) as f32 / 255.0,
            ((value >> 8) & 0xff) as f32 / 255.0,
            (value & 0xff) as f32 / 255.0,
        ]
    }

    #[must_use]
    pub fn theme(self) -> SceneTheme {
        let [r, g, b] = Self::floats(self.scene);
        let [gr, gg, gb] = Self::floats(self.scene_grid);
        SceneTheme {
            background: [r, g, b, 1.0],
            accent: Self::floats(self.scene_accent),
            dim_factor: self.dim_factor,
            grid: [gr, gg, gb, 0.55],
        }
    }
}

/// The scene theme for an appearance.
#[must_use]
pub fn scene_theme(dark: bool) -> SceneTheme {
    if dark { DARK } else { LIGHT }.theme()
}
