/** The two visual directions the header toggles between.
 *
 * Both are shipped: switching the header repaints the panels and the 3D scene
 * alike. Domain colours (good / warn / bad) are the same semantic slots in both,
 * so a red tool light stays red.
 */
export interface Palette {
  background: string;
  surface: string;
  surfaceAlt: string;
  border: string;
  text: string;
  muted: string;
  accent: string;
  accentText: string;
  good: string;
  warn: string;
  bad: string;
  info: string;
  scene: string;
}

/** "Porcelain & Ink": a bright control room, hairline borders, one teal accent. */
export const LIGHT: Palette = {
  background: "#f2f5f8",
  surface: "#ffffff",
  surfaceAlt: "#e9eef5",
  border: "#d5dee9",
  text: "#101828",
  muted: "#5b6472",
  accent: "#0f766e",
  accentText: "#ffffff",
  good: "#15803d",
  warn: "#b45309",
  bad: "#b91c1c",
  info: "#1d4ed8",
  scene: "#e8eef5",
};

/** "Cleanroom Night": the default; deep navy, luminous tool states. */
export const DARK: Palette = {
  background: "#070b11",
  surface: "#0e1620",
  surfaceAlt: "#151f2b",
  border: "#22303f",
  text: "#e6edf5",
  muted: "#8b9bb0",
  accent: "#2dd4bf",
  accentText: "#04211e",
  good: "#34d399",
  warn: "#fbbf24",
  bad: "#f87171",
  info: "#60a5fa",
  scene: "#070c13",
};

/** Defect classes, the same in both themes so a scrapped slot reads the same. */
export const DEFECT = {
  scratch: "#d9572b",
  particle: "#eb2f8c",
  resist: "#b45cf2",
  pattern: "#f2bf29",
} as const;
