"""Build the oxide-fab 3D scene and export it as GLB plus a scene manifest.

This is the only place the geometry is authored. It is an *offline* tool: the
application loads the committed `crates/oxide-ui/assets/fab/oxide-fab.glb` and
`crates/oxide-ui/assets/fab/fab-manifest.json`, beside the Rust that renders
them; neither Blender nor bpy is part of the runtime, the packages or CI.

Run from the repository root:

    blender --background --factory-startup --python tools/blender/build_fab.py

Geometry only, and a local .blend for the asset QA script (nothing else is
written except the GLB and manifest):

    blender --background --factory-startup --python tools/blender/build_fab.py \
        -- --no-render --save /tmp/oxide-fab.blend

Add `--nobake` to skip the texture bake (geometry and UVs only, untextured GLB).

The method is AnyMachine's reference-first hard-surface pipeline
(https://github.com/apekshik/anymachine, MIT): write the part list first, build
one real part at a time on a shared kit (`tools/blender/kit.py`), and let the
numbers prove the result.  Reference sheets for the four tool kinds live in
`tools/blender/refs/` with the prompts that made them (`jobs_refs.json`).
What is modelled, per process tool:

  body       skid frame, leveling feet, sheet-metal enclosure with chamfered
             corners, external corner posts, framed service doors with recessed
             leaves, hinges, handles and windows, louvred side vents, a cable
             tray and conduit, a rear junction box, an E-stop box, labels
  chamber    bolted bottom and top flanges, shell, stiffening band, domed lid
             with a centre port, viewport, gas box and feed tubes, RF match box
             with cooling fins, foreline elbow - or, for a wet bench, a box
             chamber with lid, hatch, top port and plumbing
  loadport   front-end cabinet, two docking stages with kinematic pins, door
             opener paddle on rails, presence sensors, status beacon, conduit
  panel      HMI console on a bracket: bezel, recessed screen, button row,
             E-stop, key switch, cable gland
  exhaust    vertical duct with bolted flanges, damper with a lever, hanger
             brackets and an elbow

The factory itself gets the same treatment: a raised-floor tile grid with
grooves and drain grates, plinths and panel seams on the process bands, I-beam
AMHS rails with splices, supports and bolted posts, and a FOUP and wafer with
real anatomy (door latches, purge ports, rear grille, edge profile).

Naming contract (the manifest records it, `oxide-scene` validates it):

  tool.<equipment-id>                 empty: one tool instance, the pick root
  tool.<equipment-id>.<part>          one mesh part of that tool
  factory.floor                       the floor slab
  factory.band.<step-no>              one process band
  factory.amhs.<n>                    one AMHS rail segment
  foup.template                       the FOUP prototype (not drawn as-is)
  wafer.template                      the 300 mm wafer prototype (not drawn)

One manifest node is one mesh here, so the pieces of a part are joined into
that part's mesh: `body` carries every plate, bolt and louvre of the enclosure.
`tools/blender/bake.py` then unwraps every mesh, bakes paint variation, edge
wear, grime, a paint-scale normal and an AO / roughness / metallic map, packs
all 46 tiles into one 2048 atlas set and assigns the runtime material.  The
detail that matters at playing distance is real geometry; the bake carries what
is too fine or too irregular to model (paint, wear, grime, micro-surface).  The
scene graph still gives each node one colour, which tints the texture.

Blender is Z-up with +Y away from the viewer. glTF is Y-up; the exporter's
`export_yup` maps Blender (x, y, z) to glTF (x, z, -y). Every coordinate written
to the manifest is already in glTF space, which is what the renderer and the
picking code use.
"""

from __future__ import annotations

import json
import math
import os
import sys

import bpy
from mathutils import Vector

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bake  # noqa: E402
from kit import Part, material  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT_DIR = os.path.join(ROOT, "crates", "oxide-ui", "assets", "fab")
GLB_NAME = "oxide-fab.glb"
MANIFEST_NAME = "fab-manifest.json"
PREVIEW_DIR = os.path.join("artifacts", "blender")

# Layout constants. Kept together so the scene graph and the geometry cannot
# drift.
FLOOR_HALF_X = 22.0
FLOOR_HALF_Z = 13.0
# glTF z of each band; Blender places a band at y = -z.
BAND_Z = {1: 8.5, 2: 3.0, 3: -2.5, 4: -8.0}
BAND_HALF_X = 20.0
BAND_DEPTH = 3.0
BAND_HEIGHT = 0.35
TOOL_Y = BAND_HEIGHT
RAIL_Y = 1.15
RAIL_TOP = RAIL_Y
RAIL_BOTTOM = 1.04
RING = [(-19.0, -11.0), (19.0, -11.0), (19.0, 11.0), (-19.0, 11.0)]

# step number -> the ring corner that feeds that band
STOP_POINT = {
    1: (19.0, 11.0),
    2: (-19.0, 11.0),
    3: (19.0, -11.0),
    4: (-19.0, -11.0),
}

TOOLS = [
    # id, kind, band x, band z key
    ("CLN-01", "clean", -13.0, 1),
    ("CLN-02", "clean", -1.0, 1),
    ("LIT-01", "litho", 7.0, 2),
    ("LIT-02", "litho", 16.0, 2),
    ("ETC-01", "etch", -7.0, 3),
    ("MET-01", "metrology", 7.0, 4),
]

# Per-kind envelopes in tool-local Blender space (front is -Y, up is +Z):
#   body     (width, depth, height), standing on z = 0
#   chamber  ("cyl", (radius, height) | "box", (width, depth, height), centre)
#   loadport (width, depth, height, centre)
#   panel    (width, depth, height, centre)
#   exhaust  (width, depth, height, centre)
KINDS = {
    "clean": {
        "body": (4.2, 1.5, 2.6),
        "chamber": ("box", (1.8, 1.1, 1.6), (0.9, 0.0, 3.15)),
        "loadport": (1.6, 0.9, 1.2, -1.5, -1.05, 1.35),
        "panel": (1.6, 2.0),
        "exhaust": (0.35, 0.35, 1.1, 0.9, 0.0, 4.4),
    },
    "litho": {
        "body": (5.6, 2.0, 2.8),
        "chamber": ("cyl", (1.05, 1.6), (0.6, 0.0, 3.6)),
        "loadport": (1.8, 1.0, 1.2, -2.0, -1.4, 1.2),
        "panel": (2.2, 1.55),
        "exhaust": (0.4, 0.4, 1.2, 0.6, 0.0, 5.0),
    },
    "etch": {
        "body": (4.6, 1.7, 2.7),
        "chamber": ("cyl", (0.95, 1.5), (0.6, 0.0, 3.4)),
        "loadport": (1.6, 0.9, 1.2, -1.7, -1.35, 1.1),
        "panel": (1.8, 1.4),
        "exhaust": (0.35, 0.35, 1.1, 0.6, 0.0, 4.7),
    },
    "metrology": {
        "body": (4.0, 1.6, 2.4),
        "chamber": ("box", (1.6, 1.0, 1.3), (0.7, 0.0, 2.9)),
        "loadport": (1.4, 0.8, 1.0, -1.4, -1.2, 0.95),
        "panel": (1.5, 1.25),
        "exhaust": (0.28, 0.28, 0.9, 0.2, 0.0, 4.0),
    },
}

# Explosion directions in glTF space; scaled by the manifest's distance.
EXPLODE = {
    "body": (0.0, 0.0, 0.0),
    "chamber": (0.10, 1.00, 0.10),
    "loadport": (-0.15, 0.15, 1.00),
    "panel": (0.55, 0.55, -0.55),
    "exhaust": (0.10, 1.00, 0.10),
}

MATERIALS = {
    # glTF base colour (the renderer converts to sRGB in the shader)
    "floor": ((0.085, 0.10, 0.13, 1.0), 0.05),
    "band": ((0.15, 0.17, 0.21, 1.0), 0.05),
    "rail": ((0.36, 0.42, 0.50, 1.0), 0.35),
    "body": ((0.42, 0.46, 0.52, 1.0), 0.10),
    "chamber": ((0.16, 0.30, 0.44, 1.0), 0.20),
    "loadport": ((0.42, 0.44, 0.48, 1.0), 0.35),
    "panel": ((0.10, 0.16, 0.20, 1.0), 0.10),
    "exhaust": ((0.38, 0.40, 0.44, 1.0), 0.30),
    "foup": ((0.72, 0.76, 0.80, 1.0), 0.05),
    "wafer": ((0.55, 0.62, 0.72, 1.0), 0.10),
}


# --------------------------------------------------------------------------- scene


def reset() -> None:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.unit_settings.system = "METRIC"
    scene.unit_settings.scale_length = 1.0


def make_materials() -> None:
    for name, (color, metallic) in MATERIALS.items():
        material(name, color, metallic=metallic, roughness=0.45)


def empty(name: str, loc=(0.0, 0.0, 0.0)):
    obj = bpy.data.objects.new(name, None)
    obj.empty_display_type = "PLAIN_AXES"
    obj.empty_display_size = 0.4
    obj.location = loc
    bpy.context.collection.objects.link(obj)
    return obj


def gltf_point(p: Vector) -> list[float]:
    """Blender (x, y, z) -> glTF (x, z, -y), rounded for a stable manifest."""
    return [round(p.x, 5), round(p.z, 5), round(-p.y, 5)]


def world_aabb(obj) -> dict:
    bpy.context.view_layer.update()
    corners = [obj.matrix_world @ Vector(corner) for corner in obj.bound_box]
    lo = [min(gltf_point(c)[i] for c in corners) for i in range(3)]
    hi = [max(gltf_point(c)[i] for c in corners) for i in range(3)]
    return {"min": [round(v, 4) for v in lo], "max": [round(v, 4) for v in hi]}


def triangle_count(obj) -> int:
    obj.data.calc_loop_triangles()
    return len(obj.data.loop_triangles)


# ------------------------------------------------------------------ tool parts


def door(p: Part, face: float, cx: float, width: float, z0: float, z1: float, hinged_left: bool) -> None:
    """A framed service door: recessed leaf, hinge knuckles, a handle, a window."""
    ex, ey = (1.0, 0.0, 0.0), (0.0, 0.0, 1.0)
    mid = (z0 + z1) * 0.5
    p.frame((cx, face - 0.012, mid), ex, ey, width, z1 - z0, 0.045, 0.03)
    p.box(
        (cx - width * 0.5 + 0.03, face - 0.010, z0 + 0.03),
        (cx + width * 0.5 - 0.03, face + 0.02, z1 - 0.03),
        bevel=0.006,
        skip="+y",
    )
    p.plate((cx - 0.10, face - 0.010, z1 - 0.30), ex, ey, 0.34, 0.22, proud=0.006)
    p.frame((cx - 0.10, face - 0.014, z1 - 0.30), ex, ey, 0.38, 0.26, 0.028, 0.022)
    side = -1.0 if hinged_left else 1.0
    for z in (z0 + 0.12, mid, z1 - 0.12):
        p.hinge((cx + side * (width * 0.5 - 0.035), face - 0.021, z), (0.0, 0.0, 1.0), (0.0, -1.0, 0.0), 0.11)
    grip = cx - side * (width * 0.5 - 0.14)
    p.handle((grip, face - 0.016, mid - 0.13), (grip, face - 0.016, mid + 0.13), 0.055, (0.0, -1.0, 0.0))


def louvre_panel(p: Part, face_x: float, sign: float, cy: float, cz: float, width: float, height: float) -> None:
    """A louvred vent bolted onto a side face: housing, frame, slats."""
    ex = (0.0, 1.0, 0.0) if sign > 0 else (0.0, -1.0, 0.0)
    ey = (0.0, 0.0, 1.0)
    outer = face_x + sign * 0.05
    p.box(
        (min(face_x - sign * 0.005, outer), cy - width * 0.5, cz - height * 0.5),
        (max(face_x - sign * 0.005, outer), cy + width * 0.5, cz + height * 0.5),
        bevel=0.006,
        skip="+x" if sign > 0 else "-x",
    )
    p.frame((outer, cy, cz), ex, ey, width + 0.09, height + 0.09, 0.035, 0.03)
    p.louvres((outer + sign * 0.004, cy, cz - height * 0.5 + 0.01), ex, ey, width - 0.04, height - 0.05, 9, 0.04, 0.012)


def build_body(p: Part, w: float, d: float, h: float, kind: str) -> None:
    hw, hd = w * 0.5, d * 0.5
    # Skid frame: the tool stands on a welded base, not on its panels.
    p.box((-hw + 0.06, -hd + 0.05, 0.0), (hw - 0.06, hd - 0.05, 0.10), bevel=0.014, skip="-z")
    for sx in (-1.0, 1.0):
        for sy in (-1.0, 1.0):
            x, y = sx * (hw - 0.05), sy * (hd - 0.05)
            p.box((x - 0.05, y - 0.05, 0.0), (x + 0.05, y + 0.05, 0.096), bevel=0.008, skip="-z")
            p.cyl((x, y, 0.096), (x, y, 0.128), 0.042, n=6, caps=(False, True))
    # Enclosure, inset from the skid so the frame reads as a plinth; the top
    # plate below closes it.
    ex, ey = hw - 0.02, hd - 0.02
    front, back = -ey, ey
    p.box((-ex, -ey, 0.10), (ex, ey, h), bevel=0.05, segments=3, skip="-z,+z")
    # External corner posts, with a bolt row on each exposed face.
    for sx in (-1.0, 1.0):
        for sy in (-1.0, 1.0):
            x, y = sx * ex, sy * ey
            p.box((x - 0.05, y - 0.05, 0.11), (x + 0.05, y + 0.05, h - 0.03), bevel=0.012, skip="-z")
            p.bolt_row((x, y + sy * 0.05, 0.35), (0.0, 0.0, 1.0), (0.0, sy, 0.0), 5, 0.45, 0.01)
    # Vertical panel seams across the front and back.
    columns = max(2, round(w / 0.85))
    for index in range(1, columns):
        x = -ex + 2.0 * ex * index / columns
        for face, outward in ((front, -1.0), (back, 1.0)):
            p.box(
                (x - 0.006, min(face + outward * 0.004, face - outward * 0.002), 0.14),
                (x + 0.006, max(face + outward * 0.004, face - outward * 0.002), h - 0.05),
                bevel=0.002,
                skip="+y" if outward > 0 else "-y",
            )
    # Doors, per kind.
    if kind == "clean":
        door(p, front, -1.0, 1.05, 0.28, 2.25, hinged_left=True)
        door(p, front, 1.0, 1.05, 0.28, 2.25, hinged_left=False)
    elif kind == "litho":
        door(p, front, -1.25, 1.9, 0.28, 2.45, hinged_left=True)
        door(p, front, 1.55, 0.75, 0.28, 2.45, hinged_left=False)
    elif kind == "etch":
        door(p, front, -0.75, 1.5, 0.28, 2.35, hinged_left=True)
    else:
        door(p, front, -0.65, 1.25, 0.28, 2.05, hinged_left=True)
    # Louvred vents on both sides.
    louvre_panel(p, ex, 1.0, 0.1, 1.55, 0.62, 0.78)
    louvre_panel(p, -ex, -1.0, 0.1, 1.55, 0.62, 0.78)
    # Cable tray and cables on the +X side.
    tray_z = h - 0.30
    p.box((ex - 0.01, -0.55, tray_z), (ex + 0.22, 0.55, tray_z + 0.02), bevel=0.004)
    p.box((ex + 0.22, -0.55, tray_z - 0.02), (ex + 0.24, 0.55, tray_z + 0.12), bevel=0.003)
    for y in (-0.28, 0.0, 0.28):
        p.tube([(ex + 0.13, y, tray_z + 0.02), (ex + 0.13, y, 0.2), (ex + 0.02, y, 0.12)], 0.026, n=8)
    # Rear junction box, conduits and a nameplate.
    p.box((-0.55, back - 0.09, 0.55), (0.05, back + 0.004, 0.95), bevel=0.012, skip="+y")
    p.frame((0.0, back + 0.004, 0.75), (-1.0, 0.0, 0.0), (0.0, 0.0, 1.0), 0.66, 0.46, 0.03, 0.02)
    for x in (-0.75, 0.25):
        p.cyl((x, back - 0.02, 0.22), (x, back - 0.02, 0.55), 0.045, n=10)
        p.flange((x, back - 0.02, 0.25), (0.0, 0.0, 1.0), 0.075, 0.045, 0.03, bolts=6, bolt_r=0.008)
    p.plate((0.0, back + 0.002, 1.45), (-1.0, 0.0, 0.0), (0.0, 0.0, 1.0), 0.34, 0.16, proud=0.004)
    p.plate((-hw + 0.55, front - 0.002, 0.85), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0), 0.18, 0.10, proud=0.003)
    p.plate((hw - 0.55, front - 0.002, 0.55), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0), 0.12, 0.08, proud=0.003)
    # E-stop station on the front right.
    p.box((ex - 0.48, front - 0.10, h - 0.55), (ex - 0.24, front + 0.02, h - 0.27), bevel=0.012)
    p.cyl((ex - 0.36, front - 0.105, h - 0.41), (ex - 0.36, front - 0.16, h - 0.41), 0.042, n=16)
    p.cyl((ex - 0.36, front - 0.16, h - 0.41), (ex - 0.36, front - 0.19, h - 0.41), 0.055, n=16)
    # Kick plate.
    p.box((-hw + 0.3, front - 0.012, 0.13), (hw - 0.3, front + 0.004, 0.24), bevel=0.005, skip="+y")
    # Top plate: raised rim, bolted, clear of the chamber.
    p.box((-ex + 0.03, -ey + 0.03, h - 0.02), (ex - 0.03, ey - 0.03, h + 0.01), bevel=0.008)
    p.bolt_row((-ex + 0.12, ey - 0.09, h + 0.01), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0), max(2, round(w / 0.5)), 0.5, 0.009)


def build_chamber_box(p: Part, w: float, d: float, h: float, cx: float, cz: float, kind: str) -> None:
    z0, z1 = cz - h * 0.5, cz + h * 0.5
    p.box((cx - w * 0.5, -d * 0.5, z0), (cx + w * 0.5, d * 0.5, z1), bevel=0.035, segments=3, skip="-z")
    p.box(
        (cx - w * 0.5 - 0.035, -d * 0.5 - 0.035, z1 - 0.02),
        (cx + w * 0.5 + 0.035, d * 0.5 + 0.035, z1 + 0.07),
        bevel=0.02,
    )
    p.bolt_row((cx - w * 0.5, -d * 0.5 - 0.02, z1 + 0.07), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0), max(3, round(w / 0.28)), 0.28, 0.01)
    # Front hatch with a frame.
    p.frame((cx, -d * 0.5 - 0.006, z0 + h * 0.55), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0), w * 0.62, h * 0.42, 0.035, 0.025)
    p.plate((cx, -d * 0.5 - 0.012, z0 + h * 0.55), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0), w * 0.52, h * 0.32, proud=0.004)
    # Top port with a bolted flange (the metrology column takes its place).
    if kind != "metrology":
        p.flange((cx - w * 0.22, 0.0, z1 + 0.07), (0.0, 0.0, 1.0), 0.20, 0.115, 0.035, bolts=8, bolt_r=0.01)
        p.cyl((cx - w * 0.22, 0.0, z1 + 0.05), (cx - w * 0.22, 0.0, z1 + 0.20), 0.115, n=20)
    # Gas box at the back, with three feed tubes that run up and forward over
    # the lid without converging on each other.
    p.box((cx + w * 0.10, d * 0.5, z0 + h * 0.25), (cx + w * 0.10 + 0.42, d * 0.5 + 0.22, z0 + h * 0.25 + 0.45), bevel=0.012, skip="+y,-y")
    for k in range(3):
        x = cx + w * 0.10 + 0.10 + k * 0.11
        p.tube([(x, d * 0.5 + 0.15, z0 + h * 0.25 + 0.45), (x, d * 0.5 + 0.15, z1 + 0.18), (x, d * 0.5 - 0.15, z1 + 0.18)], 0.016, n=8)
        p.cyl((x, d * 0.5 + 0.15, z0 + h * 0.25 + 0.52), (x, d * 0.5 + 0.15, z0 + h * 0.25 + 0.60), 0.03, n=10)
    # (The chamber is bolted to the body's top deck; it has no feet of its own.)
    if kind == "metrology":
        # Optics column: a lathe stack over the stage, with an objective and a camera.
        p.lathe(
            [(0.0, 0.0), (0.42, 0.0), (0.42, 0.10), (0.30, 0.16), (0.30, 0.52), (0.22, 0.58), (0.22, 0.78), (0.14, 0.84), (0.0, 0.84)],
            at=(cx + 0.30, 0.0, z1 + 0.07),
            n=28,
        )
        p.bolt_circle((cx + 0.30, 0.0, z1 + 0.09), (0.0, 0.0, 1.0), 0.34, 8, 0.011)


def build_chamber_cyl(p: Part, r: float, h: float, cx: float, cz: float) -> None:
    z0, z1 = cz - h * 0.5, cz + h * 0.5
    p.flange((cx, 0.0, z0 + 0.045), (0.0, 0.0, 1.0), r * 1.16, r * 0.74, 0.09, bolts=16, bolt_r=0.013, bolt_h=0.01)
    p.cyl((cx, 0.0, z0 + 0.06), (cx, 0.0, z1 - 0.09), r, n=36)
    p.flange((cx, 0.0, (z0 + z1) * 0.5), (0.0, 0.0, 1.0), r * 1.05, r * 0.985, 0.035)
    p.flange((cx, 0.0, z1 - 0.075), (0.0, 0.0, 1.0), r * 1.16, r * 0.74, 0.09, bolts=16, bolt_r=0.013, bolt_h=0.01, bolt_phase=0.0)
    # Domed lid with a centre port.
    p.lathe(
        [(0.0, 0.0), (r * 0.98, 0.0), (r * 0.98, 0.04), (r * 0.72, 0.10), (r * 0.34, 0.15), (r * 0.34, 0.20), (r * 0.24, 0.24), (0.0, 0.26)],
        at=(cx, 0.0, z1 - 0.03),
        n=36,
    )
    p.flange((cx, 0.0, z1 + 0.21), (0.0, 0.0, 1.0), 0.20, 0.125, 0.03, bolts=8, bolt_r=0.009)
    p.bolt_circle((cx, 0.0, z1 - 0.02), (0.0, 0.0, 1.0), r * 0.88, 12, 0.012)
    # Viewport on the front.
    vy = -r * 0.99
    p.cyl((cx, vy + 0.06, (z0 + z1) * 0.5 + 0.05), (cx, vy - 0.09, (z0 + z1) * 0.5 + 0.05), 0.145, n=24)
    p.flange((cx, vy - 0.10, (z0 + z1) * 0.5 + 0.05), (0.0, -1.0, 0.0), 0.17, 0.10, 0.035, bolts=8, bolt_r=0.008)
    p.cyl((cx, vy - 0.13, (z0 + z1) * 0.5 + 0.05), (cx, vy - 0.145, (z0 + z1) * 0.5 + 0.05), 0.10, n=24)
    # Gas box at the back with feed tubes and valves.
    gy = r * 0.98
    p.box((cx - 0.22, gy - 0.02, z0 + 0.55), (cx + 0.22, gy + 0.22, z0 + 0.95), bevel=0.012, skip="-y,+y")
    p.frame((cx, gy + 0.225, z0 + 0.75), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0), 0.40, 0.34, 0.03, 0.02)
    for k, dx in enumerate((-0.12, 0.0, 0.12)):
        x = cx + dx
        p.tube([(x, gy + 0.14, z0 + 0.95), (x, gy + 0.14, z1 + 0.10), (cx + dx, gy * 0.35, z1 + 0.10)], 0.014, n=8)
        p.cyl((x, gy + 0.14, z0 + 1.02), (x, gy + 0.14, z0 + 1.10), 0.026, n=10)
        p.cyl((x, gy + 0.14, z0 + 1.10), (x, gy + 0.14, z0 + 1.14), 0.038, n=10)
    # RF match box with cooling fins on the left.
    mx = cx - r * 0.99
    p.box((mx - 0.22, -0.24, z0 + 0.45), (mx + 0.02, 0.24, z0 + 1.05), bevel=0.014, skip="+x,-x")
    for k in range(7):
        z = z0 + 0.52 + k * 0.075
        p.box((mx - 0.24, -0.20, z), (mx - 0.22, 0.20, z + 0.012), skip="+x")
    # Foreline elbow into the pump.
    p.tube([(cx + r * 0.55, 0.0, z0 + 0.09), (cx + r * 0.55, 0.0, z0 - 0.05), (cx + r * 0.55, 0.30, z0 - 0.12)], 0.075, n=12)
    p.flange((cx + r * 0.55, 0.0, z0 + 0.09), (0.0, 0.0, 1.0), 0.12, 0.075, 0.03, bolts=8, bolt_r=0.009)


def build_chamber(p: Part, spec, kind: str) -> None:
    if spec[0] == "cyl":
        (r, h), (cx, _, cz) = spec[1], spec[2]
        build_chamber_cyl(p, r, h, cx, cz)
    else:
        (w, d, h), (cx, _, cz) = spec[1], spec[2]
        build_chamber_box(p, w, d, h, cx, cz, kind)


def build_loadport(p: Part, w: float, d: float, h: float, cx: float, cy: float, cz: float) -> None:
    """A front-end module: cabinet, docking stages, door openers, beacon."""
    z0 = TOOL_Y
    top = cz + h * 0.5 - 0.12
    p.box((cx - w * 0.5, cy - d * 0.5, z0), (cx + w * 0.5, cy + d * 0.5, top), bevel=0.03, segments=2, skip="-z")
    p.box((cx - w * 0.5 + 0.04, cy - d * 0.5 - 0.012, z0 + 0.08), (cx + w * 0.5 - 0.04, cy - d * 0.5 + 0.01, top - 0.08), bevel=0.006, skip="+y")
    p.bolt_row((cx - w * 0.5 + 0.10, cy - d * 0.5 - 0.014, z0 + 0.16), (1.0, 0.0, 0.0), (0.0, -1.0, 0.0), max(3, round(w / 0.4)), 0.4, 0.008)
    # Docking stages: one per half for a wide module, one for a narrow one.
    stages = 2 if w >= 1.5 else 1
    for index in range(stages):
        sx = cx + (index - (stages - 1) * 0.5) * (w / stages)
        stage_w = w / stages - 0.12
        y_front = cy - d * 0.5 + 0.04
        y_back = y_front + d * 0.62
        p.box((sx - stage_w * 0.5, y_front, top), (sx + stage_w * 0.5, y_back + 0.10, top + 0.05), bevel=0.008, skip="-z")
        # Port wall with the door opening, the paddle and its guide rails.
        p.box((sx - stage_w * 0.55, y_back, top), (sx + stage_w * 0.55, y_back + 0.10, top + 0.58), bevel=0.012, skip="-z")
        p.frame((sx, y_back - 0.004, top + 0.34), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0), stage_w * 0.56, 0.44, 0.03, 0.024)
        p.box((sx - stage_w * 0.19, y_back - 0.028, top + 0.16), (sx + stage_w * 0.19, y_back + 0.01, top + 0.26), bevel=0.006)
        for rail_x in (-stage_w * 0.24, stage_w * 0.24):
            p.box((sx + rail_x - 0.012, y_back - 0.036, top), (sx + rail_x + 0.012, y_back + 0.006, top + 0.50), bevel=0.003)
        # Kinematic pins on the stage plate.
        for pin_x, pin_y in ((-0.20, 0.16), (0.20, 0.16), (0.0, -0.12)):
            p.cyl((sx + pin_x, y_front + d * 0.31 + pin_y, top + 0.044), (sx + pin_x, y_front + d * 0.31 + pin_y, top + 0.075), 0.022, n=10)
        # Presence sensors either side of the stage.
        for sensor_x in (-stage_w * 0.56, stage_w * 0.56):
            p.box((sx + sensor_x - 0.018, y_front + 0.08, top + 0.044), (sx + sensor_x + 0.018, y_front + 0.13, top + 0.13), bevel=0.004)
            p.cyl((sx + sensor_x, y_front + 0.13, top + 0.09), (sx + sensor_x, y_front + 0.145, top + 0.09), 0.009, n=8)
        # Stack light on the port wall: post, base disc, three lenses, cap.
        bx = sx + stage_w * 0.42
        base_z = top + 0.58
        p.cyl((bx, y_back + 0.05, base_z), (bx, y_back + 0.05, base_z + 0.08), 0.016, n=10)
        p.cyl((bx, y_back + 0.05, base_z + 0.08), (bx, y_back + 0.05, base_z + 0.10), 0.036, n=16)
        for k in range(3):
            z = base_z + 0.10 + k * 0.046
            p.cyl((bx, y_back + 0.05, z), (bx, y_back + 0.05, z + 0.038), 0.030, n=16)
        p.cyl((bx, y_back + 0.05, base_z + 0.238), (bx, y_back + 0.05, base_z + 0.25), 0.036, n=16)
    # Front conduit, clipped to the cabinet, and a warning plate.
    p.tube([(cx + w * 0.32, cy - d * 0.5 - 0.02, z0 + 0.85), (cx + w * 0.32, cy - d * 0.5 - 0.02, z0 + 0.25), (cx + w * 0.32, cy - d * 0.5 + 0.10, z0 + 0.18)], 0.022, n=8)
    for z in (z0 + 0.35, z0 + 0.65):
        p.box((cx + w * 0.32 - 0.02, cy - d * 0.5 - 0.026, z), (cx + w * 0.32 + 0.02, cy - d * 0.5 + 0.006, z + 0.03), bevel=0.003)
    p.plate((cx - w * 0.3, cy - d * 0.5 - 0.014, z0 + 0.45), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0), 0.20, 0.11, proud=0.004)


def build_panel(p: Part, cx: float, cz: float, body_front: float) -> None:
    """An HMI console on a bracket: bezel, recessed screen, buttons, E-stop."""
    half_w = 0.44
    half_h = 0.32
    cy = body_front - 0.45
    p.box((cx - 0.07, cy + 0.08, cz - 0.07), (cx + 0.07, body_front + 0.02, cz + 0.07), bevel=0.008)
    p.box((cx - half_w, cy - 0.10, cz - half_h), (cx + half_w, cy + 0.10, cz + half_h), bevel=0.022, segments=3)
    face = cy - 0.10
    p.frame((cx, face - 0.017, cz + 0.04), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0), 0.78, 0.50, 0.04, 0.028)
    p.plate((cx, face - 0.012, cz + 0.04), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0), 0.74, 0.46, proud=0.004)
    for k in range(5):
        x = cx - 0.24 + k * 0.10
        p.cyl((x, face + 0.004, cz - 0.20), (x, face - 0.022, cz - 0.20), 0.016, n=12)
    p.box((cx + 0.28, face - 0.03, cz - 0.24), (cx + 0.38, face, cz - 0.14), bevel=0.008)
    p.cyl((cx + 0.33, face - 0.026, cz - 0.19), (cx + 0.33, face - 0.075, cz - 0.19), 0.032, n=16)
    p.cyl((cx + 0.33, face - 0.075, cz - 0.19), (cx + 0.33, face - 0.095, cz - 0.19), 0.042, n=16)
    p.cyl((cx - 0.34, face + 0.006, cz - 0.20), (cx - 0.34, face - 0.05, cz - 0.20), 0.022, n=12)
    p.box((cx - 0.40, face - 0.05, cz - 0.215), (cx - 0.28, face - 0.045, cz - 0.185), bevel=0.0015)
    # Cable gland and a cable routed back to the tool.
    p.cyl((cx, cy + 0.04, cz - half_h - 0.02), (cx, cy + 0.04, cz - half_h - 0.10), 0.030, n=12)
    p.tube(
        [
            (cx, cy + 0.04, cz - half_h - 0.10),
            (cx, cy + 0.30, cz - half_h - 0.16),
            (cx, body_front + 0.01, cz - half_h - 0.16),
        ],
        0.022,
        n=8,
    )


def build_exhaust(p: Part, w: float, d: float, h: float, cx: float, cy: float, cz: float) -> None:
    """A process exhaust: flanged duct, damper with a lever, swept elbow."""
    z0, z1 = cz - h * 0.5, cz + h * 0.5
    radius = min(w, d) * 0.5
    bend = 0.17
    p.flange((cx, cy, z0 + 0.03), (0.0, 0.0, 1.0), radius * 1.30, radius * 0.75, 0.06, bolts=8, bolt_r=0.009)
    p.cyl((cx, cy, z0 + 0.05), (cx, cy, z1 - 0.22), radius, n=18)
    for k in range(3):
        z = z0 + 0.16 + k * (h - 0.40) / 3.0
        p.flange((cx, cy, z), (0.0, 0.0, 1.0), radius * 1.22, radius * 0.78, 0.028, bolts=8, bolt_r=0.008)
    # Damper spindle and lever.
    p.cyl((cx - radius * 1.18, cy, z0 + h * 0.42), (cx + radius * 1.18, cy, z0 + h * 0.42), 0.022, n=10)
    p.box((cx + radius * 1.18 - 0.01, cy - 0.008, z0 + h * 0.42), (cx + radius * 1.18 + 0.14, cy + 0.008, z0 + h * 0.46), bevel=0.004)
    # A swept 90-degree elbow into a horizontal outlet.
    arc = []
    for step in range(5):
        angle = math.pi * (1.0 - 0.5 * step / 4.0)
        arc.append((cx, cy + bend + bend * math.cos(angle), z1 - 0.22 + bend * math.sin(angle)))
    p.tube([(cx, cy, z1 - 0.30)] + arc + [(cx, cy + 0.52, z1 - 0.22 + bend)], radius, n=14)
    p.flange((cx, cy + 0.52, z1 - 0.22 + bend), (0.0, 1.0, 0.0), radius * 1.24, radius * 0.78, 0.03, bolts=8, bolt_r=0.008)


# ---------------------------------------------------------------- factory parts


def build_floor(p: Part) -> None:
    half_x, half_z = FLOOR_HALF_X, FLOOR_HALF_Z
    columns = max(1, round(2.0 * half_x / 1.0))
    rows = max(1, round(2.0 * half_z / 1.0))
    tile_x, tile_z = 2.0 * half_x / columns, 2.0 * half_z / rows
    gap = 0.014
    # The raised floor: a solid sub-floor with a grid of tiles on top, the gaps
    # reading as the panel joints.
    p.box((-half_x, -half_z, -0.25), (half_x, half_z, -0.03), bevel=0.02)
    for column in range(columns):
        for row in range(rows):
            x0 = -half_x + column * tile_x
            y0 = -half_z + row * tile_z
            p.box(
                (x0 + gap * 0.5, y0 + gap * 0.5, -0.03),
                (x0 + tile_x - gap * 0.5, y0 + tile_z - gap * 0.5, 0.0),
                skip="-z",
            )
    # Perimeter curb.
    curb, height = 0.18, 0.06
    p.box((-half_x, -half_z, 0.0), (half_x, -half_z + curb, height), bevel=0.01, skip="-z")
    p.box((-half_x, half_z - curb, 0.0), (half_x, half_z, height), bevel=0.01, skip="-z")
    p.box((-half_x, -half_z + curb, 0.0), (-half_x + curb, half_z - curb, height), bevel=0.01, skip="-z")
    p.box((half_x - curb, -half_z + curb, 0.0), (half_x, half_z - curb, height), bevel=0.01, skip="-z")
    # Drain grates in the aisles between the bands.
    for gx, gz in ((-8.0, 5.75), (5.0, 0.25), (-4.0, -5.25)):
        y = -gz
        p.frame((gx, y, 0.004), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0), 0.56, 0.56, 0.05, 0.016)
        for k in range(7):
            offset = -0.22 + k * 0.075
            p.box((gx + offset - 0.012, y - 0.24, 0.004), (gx + offset + 0.012, y + 0.24, 0.012), skip="-z")


def build_band(p: Part, z_gltf: float) -> None:
    y = -z_gltf
    p.box((-BAND_HALF_X, y - BAND_DEPTH * 0.5, 0.0), (BAND_HALF_X, y + BAND_DEPTH * 0.5, BAND_HEIGHT), bevel=0.03, segments=2, skip="-z")
    # End caps and a recessed kick channel.
    for sx in (-1.0, 1.0):
        x = sx * (BAND_HALF_X - 0.10)
        p.box((x - 0.10, y - BAND_DEPTH * 0.5 - 0.02, 0.02), (x + 0.10, y + BAND_DEPTH * 0.5 + 0.02, BAND_HEIGHT + 0.01), bevel=0.012)
    for face in (-1.0, 1.0):
        yy = y + face * BAND_DEPTH * 0.5
        p.box(
            (-BAND_HALF_X + 0.2, min(yy, yy + face * 0.012), 0.06),
            (BAND_HALF_X - 0.2, max(yy, yy + face * 0.012), 0.14),
            bevel=0.004,
            skip="+y" if face > 0 else "-y",
        )
        # Panel seams along the band.
        segments = 20
        for index in range(1, segments):
            x = -BAND_HALF_X + 2.0 * BAND_HALF_X * index / segments
            p.box(
                (x - 0.006, min(yy + face * 0.004, yy - face * 0.003), 0.16),
                (x + 0.006, max(yy + face * 0.004, yy - face * 0.003), BAND_HEIGHT - 0.03),
                bevel=0.002,
            )
        p.bolt_row((-BAND_HALF_X + 0.25, yy + face * 0.018, BAND_HEIGHT), (1.0, 0.0, 0.0), (0.0, face, 0.0), 40, 1.0, 0.008)
    # Top deck seams at the shoulders, clear of the tool footprint.
    for offset in (-0.80, 0.80):
        p.box((-BAND_HALF_X + 0.15, y + offset - 0.006, BAND_HEIGHT), (BAND_HALF_X - 0.15, y + offset + 0.006, BAND_HEIGHT + 0.003), skip="-z")


def _beam(p: Part, a: tuple[float, float], b: tuple[float, float]) -> None:
    """An I-beam rail between two (x, y) points, with splices and supports."""
    (x0, y0), (x1, y1) = a, b
    along_x = abs(x1 - x0) >= abs(y1 - y0)
    length = math.hypot(x1 - x0, y1 - y0)
    mx, my = (x0 + x1) * 0.5, (y0 + y1) * 0.5
    width, web, flange = 0.22, 0.045, 0.03

    def bar(z0: float, z1: float, span: float) -> None:
        if along_x:
            p.box((mx - length * 0.5, my - span * 0.5, z0), (mx + length * 0.5, my + span * 0.5, z1), bevel=0.004)
        else:
            p.box((mx - span * 0.5, my - length * 0.5, z0), (mx + span * 0.5, my + length * 0.5, z1), bevel=0.004)

    bar(RAIL_TOP - flange, RAIL_TOP, width)
    bar(RAIL_BOTTOM + flange - 0.005, RAIL_TOP - flange + 0.005, web)
    bar(RAIL_BOTTOM, RAIL_BOTTOM + flange, width * 0.9)
    # Splice plates and their bolt pairs every 4 m.
    count = max(1, int(length // 4.0))
    for index in range(1, count + 1):
        t = index / (count + 1)
        sx = x0 + (x1 - x0) * t
        sy = y0 + (y1 - y0) * t
        for side in (-1.0, 1.0):
            if along_x:
                p.box((sx - 0.16, sy + side * web * 0.5 - 0.008, RAIL_BOTTOM + 0.04), (sx + 0.16, sy + side * web * 0.5 + 0.008, RAIL_TOP - 0.04), bevel=0.003)
            else:
                p.box((sx + side * web * 0.5 - 0.008, sy - 0.16, RAIL_BOTTOM + 0.04), (sx + side * web * 0.5 + 0.008, sy + 0.16, RAIL_TOP - 0.04), bevel=0.003)
            for offset in (-0.10, 0.10):
                if along_x:
                    p.bolt((sx + offset, sy + side * web * 0.5, RAIL_BOTTOM + 0.07), (0.0, side, 0.0), 0.008)
                else:
                    p.bolt((sx + side * web * 0.5, sy + offset, RAIL_BOTTOM + 0.07), (side, 0.0, 0.0), 0.008)
    # Supports under the beam every 6.4 m.
    supports = max(2, int(length // 6.4) + 1)
    for index in range(supports):
        t = (index + 0.5) / supports
        sx = x0 + (x1 - x0) * t
        sy = y0 + (y1 - y0) * t
        p.box((sx - 0.055, sy - 0.055, 0.0), (sx + 0.055, sy + 0.055, RAIL_BOTTOM), bevel=0.006, skip="-z")
        p.box((sx - 0.16, sy - 0.16, 0.0), (sx + 0.16, sy + 0.16, 0.024), bevel=0.006, skip="-z")
        p.box((sx - 0.13, sy - 0.13, RAIL_BOTTOM - 0.03), (sx + 0.13, sy + 0.13, RAIL_BOTTOM), bevel=0.006)
        for dx, dy in ((-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)):
            p.bolt((sx + dx * 0.11, sy + dy * 0.11, 0.024), (0.0, 0.0, 1.0), 0.01)


def build_rail(p: Part, a: tuple[float, float], b: tuple[float, float]) -> None:
    """One AMHS rail run; `a` and `b` are glTF (x, z) ring points."""
    _beam(p, (a[0], -a[1]), (b[0], -b[1]))


def build_post(p: Part, point: tuple[float, float]) -> None:
    """A bolted corner post: base plate, column, saddle, gussets."""
    x, z = point
    y = -z
    p.box((x - 0.17, y - 0.17, 0.0), (x + 0.17, y + 0.17, 0.026), bevel=0.008, skip="-z")
    p.box((x - 0.075, y - 0.075, 0.026), (x + 0.075, y + 0.075, RAIL_BOTTOM), bevel=0.01, skip="-z")
    p.box((x - 0.15, y - 0.15, RAIL_BOTTOM), (x + 0.15, y + 0.15, RAIL_BOTTOM + 0.03), bevel=0.008)
    for dx, dy in ((-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)):
        p.bolt((x + dx * 0.12, y + dy * 0.12, 0.026), (0.0, 0.0, 1.0), 0.012)
        # A gusset in the plane of each base edge.
        if dx > 0:
            p.prism([(0.0, 0.0), (0.20, 0.0), (0.0, 0.20)], (1.0, 0.0, 0.0), (0.0, 0.0, 1.0), -0.008, 0.008, at=(x + 0.075, y + dy * 0.075, 0.026))
        else:
            p.prism([(0.0, 0.0), (0.20, 0.0), (0.0, 0.20)], (-1.0, 0.0, 0.0), (0.0, 0.0, 1.0), -0.008, 0.008, at=(x - 0.075, y + dy * 0.075, 0.026))
    p.bolt_row((x - 0.10, y - 0.10, 0.20), (0.0, 0.0, 1.0), (0.0, -1.0, 0.0), 5, 0.16, 0.008)


# --------------------------------------------------------------------- templates


def build_foup(p: Part) -> None:
    """A FOUP: shell, recessed door with latches, purge ports, rear grille."""
    p.box((-0.47, -0.42, 0.0), (0.47, 0.42, 0.06), bevel=0.012, skip="-z")
    p.box((-0.525, -0.475, 0.06), (0.525, 0.475, 0.95), bevel=0.07, segments=4, skip="-z")
    p.box((-0.45, -0.40, 0.95), (0.45, 0.40, 1.01), bevel=0.025)
    p.box((-0.18, -0.14, 1.01), (0.18, 0.14, 1.045), bevel=0.012, skip="+z")
    # Front door.
    face = -0.475
    p.frame((0.0, face - 0.006, 0.50), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0), 0.86, 0.80, 0.04, 0.03)
    p.box((-0.40, face - 0.016, 0.14), (0.40, face + 0.01, 0.88), bevel=0.006, skip="+y")
    p.box((-0.10, face - 0.026, 0.46), (0.10, face - 0.016, 0.62), bevel=0.006, skip="+y")
    for x in (-0.20, 0.20):
        p.cyl((x, face - 0.018, 0.54), (x, face - 0.040, 0.54), 0.030, n=14)
        p.box((x - 0.045, face - 0.042, 0.70), (x + 0.045, face - 0.036, 0.74), bevel=0.0015)
    # Side handles, bumpers and the flat-orientation notch mark.
    for sx in (-1.0, 1.0):
        p.plate((sx * 0.527, 0.0, 0.55), (0.0, sx, 0.0), (0.0, 0.0, 1.0), 0.30, 0.10, proud=0.004)
        p.handle((sx * 0.527, -0.10, 0.60), (sx * 0.527, 0.10, 0.60), 0.035, (sx, 0.0, 0.0), 0.014)
        for sy in (-1.0, 1.0):
            p.box((sx * 0.50 - 0.03, sy * 0.45 - 0.03, 0.10), (sx * 0.50 + 0.03, sy * 0.45 + 0.03, 0.20), bevel=0.008)
    # Purge ports and a rear grille.
    for x in (-0.14, 0.14):
        p.cyl((x, -0.44, 0.10), (x, -0.52, 0.10), 0.038, n=14)
        p.cyl((x, -0.52, 0.10), (x, -0.55, 0.10), 0.026, n=14)
    for k in range(6):
        z = 0.30 + k * 0.10
        p.box((-0.30, 0.475, z), (0.30, 0.492, z + 0.05), bevel=0.004, skip="-y")
    p.plate((0.0, face - 0.017, 0.94), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0), 0.28, 0.08, proud=0.003)


def build_foup_handle(p: Part) -> None:
    """The FOUP's top handle and robotic flange, a separate part."""
    p.tube([(-0.30, 0.0, 1.03), (-0.30, 0.0, 1.12), (0.30, 0.0, 1.12), (0.30, 0.0, 1.03)], 0.030, n=10)
    p.box((-0.09, -0.09, 1.045), (0.09, 0.09, 1.075), bevel=0.008)


def build_wafer(p: Part) -> None:
    """A 300 mm wafer: beveled rim and a flat face."""
    p.lathe(
        [(0.0, 0.0), (0.342, 0.0), (0.348, 0.003), (0.348, 0.011), (0.342, 0.014), (0.0, 0.014)],
        at=(0.0, 0.0, 0.013),
        n=48,
    )


# --------------------------------------------------------------------- assembly


def build_static() -> list[dict]:
    nodes = []
    floor = Part("factory.floor")
    build_floor(floor)
    nodes.append({"node": floor.name, "role": "floor"})
    floor.finish(bpy.data.materials["floor"])

    for step_no, z in BAND_Z.items():
        band = Part(f"factory.band.{step_no}")
        build_band(band, z)
        nodes.append({"node": band.name, "role": "process_band", "step_no": step_no})
        band.finish(bpy.data.materials["band"])

    for index, (a, b) in enumerate([(0, 1), (1, 2), (2, 3), (3, 0)]):
        rail = Part(f"factory.amhs.{index}")
        build_rail(rail, RING[a], RING[b])
        nodes.append({"node": rail.name, "role": "amhs_rail"})
        rail.finish(bpy.data.materials["rail"])

    for index, point in enumerate(RING):
        post = Part(f"factory.amhs.post.{index}")
        build_post(post, point)
        nodes.append({"node": post.name, "role": "amhs_post"})
        post.finish(bpy.data.materials["rail"])
    return nodes


def build_tool(equipment_id: str, kind: str, x: float, band: int) -> dict:
    root = empty(f"tool.{equipment_id}", (x, -BAND_Z[band], TOOL_Y))
    spec = KINDS[kind]
    parts = []

    builders = (
        ("body", lambda p: build_body(p, *spec["body"], kind)),
        ("chamber", lambda p: build_chamber(p, spec["chamber"], kind)),
        ("loadport", lambda p: build_loadport(p, *spec["loadport"])),
        ("panel", lambda p: build_panel(p, *spec["panel"], body_front=-spec["body"][1] * 0.5 + 0.02)),
        ("exhaust", lambda p: build_exhaust(p, *spec["exhaust"])),
    )
    for name, build in builders:
        part = Part(f"tool.{equipment_id}.{name}")
        build(part)
        obj = part.finish(bpy.data.materials[name], parent=root)
        parts.append(
            {
                "node": obj.name,
                "part": name,
                "explode": list(EXPLODE[name]),
                "aabb": world_aabb(obj),
                "triangles": triangle_count(obj),
            }
        )
    bpy.context.view_layer.update()
    anchor = gltf_point(Vector((root.location.x, root.location.y, root.location.z)))
    return {
        "equipment_id": equipment_id,
        "kind": kind,
        "root": root.name,
        "band": band,
        "anchor": [round(v, 4) for v in anchor],
        "parts": parts,
    }


def build_templates() -> tuple[list[dict], list[dict]]:
    foup = Part("foup.template")
    build_foup(foup)
    foup_obj = foup.finish(bpy.data.materials["foup"])
    handle = Part("foup.template.handle")
    build_foup_handle(handle)
    handle_obj = handle.finish(bpy.data.materials["rail"], parent=foup_obj)
    foup_parts = [
        {"node": foup_obj.name, "role": "body", "aabb": world_aabb(foup_obj)},
        {"node": handle_obj.name, "role": "handle", "aabb": world_aabb(handle_obj)},
    ]
    wafer = Part("wafer.template")
    build_wafer(wafer)
    wafer_obj = wafer.finish(bpy.data.materials["wafer"])
    return foup_parts, [{"node": wafer_obj.name, "role": "wafer", "aabb": world_aabb(wafer_obj)}]



# ----------------------------------------------------------------------- preview


def build_preview():
    """Camera and lights for the offline preview renders (not exported)."""
    scene = bpy.context.scene
    camera_data = bpy.data.cameras.new("preview.camera")
    camera = bpy.data.objects.new("preview.camera", camera_data)
    scene.collection.objects.link(camera)
    target = bpy.data.objects.new("preview.target", None)
    target.location = (0.0, 0.0, 2.0)
    scene.collection.objects.link(target)
    track = camera.constraints.new(type="TRACK_TO")
    track.name = "preview.track"
    track.target = target
    track.track_axis = "TRACK_NEGATIVE_Z"
    track.up_axis = "UP_Y"
    camera_data.lens = 50.0
    scene.camera = camera

    sun_data = bpy.data.lights.new("preview.sun", type="SUN")
    sun_data.energy = 3.2
    sun = bpy.data.objects.new("preview.sun", sun_data)
    sun.rotation_euler = (math.radians(50.0), math.radians(8.0), math.radians(35.0))
    scene.collection.objects.link(sun)

    area_data = bpy.data.lights.new("preview.area", type="AREA")
    area_data.energy = 900.0
    area_data.shape = "DISK"
    area_data.size = 18.0
    area = bpy.data.objects.new("preview.area", area_data)
    area.location = (-10.0, -8.0, 16.0)
    scene.collection.objects.link(area)

    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 32
    scene.cycles.use_denoising = False
    scene.render.resolution_x = 1600
    scene.render.resolution_y = 900
    scene.render.film_transparent = False
    scene.world = bpy.data.worlds.new("preview.world")
    background = next(
        (node for node in scene.world.node_tree.nodes if node.type == "BACKGROUND"), None
    )
    if background is not None:
        background.inputs[0].default_value = (0.035, 0.042, 0.055, 1.0)
        background.inputs[1].default_value = 0.7
    return camera


def render(camera, name: str, location, lens: float, target=(0.0, 0.0, 2.0)) -> None:
    scene = bpy.context.scene
    camera.location = location
    camera.data.lens = lens
    tracker = camera.constraints.get("preview.track")
    if tracker is not None and tracker.target is not None:
        tracker.target.location = target
    os.makedirs(os.path.join(ROOT, PREVIEW_DIR), exist_ok=True)
    scene.render.filepath = os.path.join(ROOT, PREVIEW_DIR, name)
    bpy.ops.render.render(write_still=True)


def export(glb_path: str):
    props = bpy.ops.export_scene.gltf.get_rna_type().properties.keys()
    kwargs = {
        "filepath": glb_path,
        "export_format": "GLB",
        "use_selection": False,
        "export_apply": True,
        "export_yup": True,
        "export_cameras": False,
        "export_lights": False,
        "export_animations": False,
        "export_extras": False,
        "export_texcoords": True,
        "export_normals": True,
        "export_materials": "EXPORT",
    }
    bpy.ops.export_scene.gltf(**{k: v for k, v in kwargs.items() if k in props})


def main() -> int:
    reset()
    make_materials()
    static = build_static()
    tools = [build_tool(*spec) for spec in TOOLS]
    foup_parts, wafer_parts = build_templates()

    if "--nobake" not in sys.argv:
        meshes = [obj for obj in bpy.data.objects if obj.type == "MESH"]
        atlas_dir = os.path.join(ROOT, PREVIEW_DIR, "atlas")
        os.makedirs(atlas_dir, exist_ok=True)
        bake.bake_scene(meshes, MATERIALS, atlas_dir)
        print(f"baked {len(meshes)} meshes into {bake.ATLAS}px atlases")

    manifest = {
        "schema": 1,
        "model": GLB_NAME,
        "coordinate_system": "gltf-y-up",
        "units": "meters",
        "floor": {
            "half_x": FLOOR_HALF_X,
            "half_z": FLOOR_HALF_Z,
        },
        "bands": [
            {
                "step_no": step_no,
                "name": name,
                "z": BAND_Z[step_no],
                "y": BAND_HEIGHT,
                "half_x": BAND_HALF_X,
            }
            for step_no, name in [(1, "clean"), (2, "litho"), (3, "etch"), (4, "metrology")]
        ],
        "static": static,
        "tools": tools,
        "templates": {
            "foup": foup_parts,
            "wafer": wafer_parts,
        },
        "amhs": {
            "points": [list(p) for p in RING],
            "loop": True,
            "height": RAIL_Y,
            "stops": {str(step): list(point) for step, point in STOP_POINT.items()},
        },
        "explode_distance": 2.0,
        "pick_padding": 0.08,
    }

    os.makedirs(OUT_DIR, exist_ok=True)
    total = 0
    for entry in static:
        obj = bpy.data.objects[entry["node"]]
        tris = triangle_count(obj)
        total += tris
        print(f"  {entry['node']:<28} {tris:>6} tris")
    for tool in tools:
        for part in tool["parts"]:
            total += part["triangles"]
            print(f"  {tool['equipment_id']:<8} {part['part']:<10} {part['triangles']:>6} tris")
            del part["triangles"]
    print(f"total {total} triangles, {len(bpy.data.objects)} objects")

    export(os.path.join(OUT_DIR, GLB_NAME))
    with open(os.path.join(OUT_DIR, MANIFEST_NAME), "w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2, sort_keys=False)
        handle.write("\n")

    if "--save" in sys.argv:
        # A local .blend for the asset QA script; never committed.
        bpy.ops.wm.save_as_mainfile(filepath=sys.argv[sys.argv.index("--save") + 1])

    if "--no-render" not in sys.argv:
        camera = build_preview()
        render(camera, "fx-overview.png", (26.0, -30.0, 20.0), 46.0)
        render(camera, "fx-tool-closeup.png", (-18.5, -13.5, 4.6), 55.0, target=(-13.0, -8.2, 2.4))
        render(camera, "fx-tool-front.png", (7.0, -13.5, 3.8), 50.0, target=(7.0, -4.0, 2.2))
    print(
        "wrote",
        os.path.join(OUT_DIR, GLB_NAME),
        os.path.join(OUT_DIR, MANIFEST_NAME),
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
