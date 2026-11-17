# ADR 008 — Visual direction: two shipped themes, four proposed

- **Status:** accepted
- **Date:** 2026-11-17
- **Decision:** implement **two of the four proposed directions** — dark
  "Cleanroom Night" (default) and light "Porcelain & Ink" — switchable at run
  time from the header. The two unshipped candidates stay documented in
  [the visual proposals](../design/visual-proposals.md).

## Why two themes rather than one

The brief asks for at least one light and one dark candidate and for the visual
direction to be chosen by the user. Since the session must not stop for that
choice, both are implemented: the user selects at run time instead of choosing a
document. The proposals document shows all four with screenshots and the reasons
the other two were not built.

## What the direction fixes

- **One accent, three semantic state colours.** Teal/amber carries identity;
  green/amber/red carry tool and lot state; every semantic colour is reused by
  the 3D view and the tables.
- **Type scale** 10.5 / 11.5 / 12.5 / 13.5 px; weight, not capitals, marks
  hierarchy.
- **Spacing** in 4 px steps; 10 px card radius, 5–7 px row radius.
- **The scene follows the theme.** The GLB's materials are shared; a light theme
  multiplies the floor/bands/rails by 4.2× so the same geometry reads on a light
  background. No second model, no baked lightmap.
- **No colour-only signals.** The three tool states have a self-lit cube in the
  scene, the state word and the legend; defects have a class and a text.

## Accessibility and interaction

- `减少动效` removes the FOUP interpolation only; it never removes a feature
  (A-11).
- `标签密度` hides the source/formula line on cards, not the values.
- Level navigation is also reachable from buttons (L0–L3, 退回, 复位视角), so the
  three-level drill-down does not require a pointer; the scene is also pickable
  for the pointer path.
- Contrast: body text and state words keep ≥ 4.5:1 against their surfaces in
  both themes; the accent is reserved for interactive/selected elements, not for
  small body text.

## Consequences

- `palette.rs` is the single colour source; `theme.json` keeps the font family
  and the radius so native controls inherit the same type.
- The scene's light/dark choice is inferred from the palette background
  luminance, so a future theme needs no renderer change.
- Screenshots of both themes are committed in `docs/screenshots/`.
