"""Build the oxide-fab 3D scene and export it as GLB plus a scene manifest.

This is the only place the geometry is authored. It is an *offline* tool: the
application loads the committed `assets/fab/oxide-fab.glb` and
`assets/fab/fab-manifest.json`; neither Blender nor bpy is part of the runtime,
the packages or CI.

Run from the repository root:

    blender --background --factory-startup --python tools/blender/build_fab.py

The script is deterministic: no randomness, no timestamps in the outputs, so the
committed GLB and manifest are reproducible byte for byte (the GLB producer
string and Blender's float noise aside — the manifest is exact).

Naming contract (the manifest records it, `oxide-scene` validates it):

  tool.<equipment-id>                 empty: one tool instance, the pick root
  tool.<equipment-id>.<part>          one mesh part of that tool
  factory.floor                       the floor slab
  factory.band.<step-no>              one process band
  factory.amhs.<n>                    one AMHS rail segment
  foup.template                       the FOUP prototype (not drawn as-is)
  wafer.template                      the 300 mm wafer prototype (not drawn)

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

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT_DIR = os.path.join(ROOT, "assets", "fab")
GLB_NAME = "oxide-fab.glb"
MANIFEST_NAME = "fab-manifest.json"
PREVIEW_DIR = os.path.join("artifacts", "blender")

# Geometry constants. Kept together so the layout the scene graph computes and
# the geometry the model carries cannot drift.
FLOOR_HALF_X = 22.0
FLOOR_HALF_Z = 13.0
# glTF z of each band: the manifest, the scene graph and the renderer all
# speak glTF space. Blender places a band at y = -z.
BAND_Z = {1: 8.5, 2: 3.0, 3: -2.5, 4: -8.0}
BAND_HALF_X = 20.0
BAND_DEPTH = 3.0
BAND_HEIGHT = 0.35
TOOL_Y = BAND_HEIGHT
RAIL_Y = 1.15
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

# Per-kind proportions (width, depth, height) of the body, and the extra parts.
# Each entry is (name, kind, size, local location in gltf space, material).
TOOL_SHAPES = {
    "clean": {
        "body": ((4.2, 1.5, 2.6), (0.0, 1.3, 0.0)),
        "chamber": ((1.8, 1.1, 1.6), (0.9, 3.15, 0.0)),
        "loadport": ((1.6, 0.9, 1.2), (-1.5, 1.35, 1.05)),
        "panel": ((0.9, 0.9, 0.25), (1.6, 2.0, 1.35)),
        "exhaust": ((0.35, 0.35, 1.1), (0.9, 4.4, 0.0)),
    },
    "litho": {
        "body": ((5.6, 2.0, 2.8), (0.0, 1.4, 0.0)),
        "chamber": ((2.4, 1.4, 1.9), (0.6, 3.6, 0.0)),
        "loadport": ((1.8, 1.0, 1.2), (-2.0, 1.4, 1.2)),
        "panel": ((1.0, 1.0, 0.25), (2.2, 2.2, 1.55)),
        "exhaust": ((0.4, 0.4, 1.2), (0.6, 5.0, 0.0)),
    },
    "etch": {
        "body": ((4.6, 1.7, 2.7), (0.0, 1.35, 0.0)),
        "chamber": ((2.0, 1.2, 1.7), (0.6, 3.4, 0.0)),
        "loadport": ((1.6, 0.9, 1.2), (-1.7, 1.35, 1.1)),
        "panel": ((0.9, 0.9, 0.25), (1.8, 2.1, 1.4)),
        "exhaust": ((0.35, 0.35, 1.1), (0.6, 4.7, 0.0)),
    },
    "metrology": {
        "body": ((4.0, 1.6, 2.4), (0.0, 1.25, 0.0)),
        "chamber": ((1.6, 1.0, 1.3), (0.7, 2.9, 0.0)),
        "loadport": ((1.4, 0.8, 1.0), (-1.4, 1.2, 0.95)),
        "panel": ((0.8, 0.8, 0.22), (1.5, 1.9, 1.25)),
        "exhaust": ((0.28, 0.28, 0.9), (0.7, 4.0, 0.0)),
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
    # glTF base colour (linear-ish; the renderer converts to sRGB in the shader)
    "floor": (0.085, 0.10, 0.13, 1.0),
    "band": (0.15, 0.17, 0.21, 1.0),
    "rail": (0.36, 0.42, 0.50, 1.0),
    "body": (0.42, 0.46, 0.52, 1.0),
    "chamber": (0.16, 0.30, 0.44, 1.0),
    "loadport": (0.42, 0.44, 0.48, 1.0),
    "panel": (0.10, 0.16, 0.20, 1.0),
    "exhaust": (0.38, 0.40, 0.44, 1.0),
    "foup": (0.72, 0.76, 0.80, 1.0),
    "wafer": (0.55, 0.62, 0.72, 1.0),
}


def reset() -> None:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.unit_settings.system = "METRIC"
    scene.unit_settings.scale_length = 1.0


def material(name: str):
    mat = bpy.data.materials.get(name)
    if mat is not None:
        return mat
    color = MATERIALS[name]
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    if bsdf is not None:
        bsdf.inputs["Base Color"].default_value = color
        bsdf.inputs["Metallic"].default_value = 0.25 if name in {"rail", "loadport"} else 0.05
        bsdf.inputs["Roughness"].default_value = 0.45
    mat.diffuse_color = color
    return mat


def cube(name: str, size, loc, mat_name: str, parent=None):
    """A box whose mesh is baked in world units (scale applied)."""
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=(0.0, 0.0, 0.0))
    obj = bpy.context.active_object
    obj.name = name
    obj.scale = (size[0], size[1], size[2])
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    obj.location = loc
    obj.data.materials.append(material(mat_name))
    if parent is not None:
        # The root's world transform is the tool's anchor; the part location is
        # local to it (no parent-inverse compensation, or every part would end
        # up at its local coordinates in world space).
        obj.parent = parent
    return obj


def cylinder(name: str, radius: float, depth: float, loc, mat_name: str, parent=None):
    bpy.ops.mesh.primitive_cylinder_add(
        vertices=28, radius=radius, depth=depth, location=(0.0, 0.0, 0.0)
    )
    obj = bpy.context.active_object
    obj.name = name
    obj.location = loc
    obj.data.materials.append(material(mat_name))
    if parent is not None:
        obj.parent = parent
    return obj


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


def build_static():
    """Floor, bands and the AMHS ring; returns the manifest's static nodes.

    Blender is Z-up: the floor plane spans X and Y, heights are Z.
    """
    nodes = []
    floor = cube(
        "factory.floor",
        (FLOOR_HALF_X * 2.0, FLOOR_HALF_Z * 2.0, 0.25),
        (0.0, 0.0, -0.125),
        "floor",
    )
    nodes.append({"node": floor.name, "role": "floor"})
    for step_no, z in BAND_Z.items():
        band = cube(
            f"factory.band.{step_no}",
            (BAND_HALF_X * 2.0, BAND_DEPTH, BAND_HEIGHT),
            (0.0, -z, BAND_HEIGHT / 2.0),
            "band",
        )
        nodes.append({"node": band.name, "role": "process_band", "step_no": step_no})
    # Ring rails: four long beams, plus posts at the corners.
    for index, (a, b) in enumerate([(0, 1), (1, 2), (2, 3), (3, 0)]):
        (xa, za), (xb, zb) = RING[a], RING[b]
        cx, cy = (xa + xb) / 2.0, -(za + zb) / 2.0
        length = math.hypot(xb - xa, zb - za)
        along_x = abs(xb - xa) > abs(zb - za)
        size = (length, 0.18, 0.18) if along_x else (0.18, length, 0.18)
        rail = cube(f"factory.amhs.{index}", size, (cx, cy, RAIL_Y), "rail")
        nodes.append({"node": rail.name, "role": "amhs_rail"})
    for index, (x, z) in enumerate(RING):
        post = cube(
            f"factory.amhs.post.{index}", (0.16, 0.16, RAIL_Y), (x, -z, RAIL_Y / 2.0), "rail"
        )
        nodes.append({"node": post.name, "role": "amhs_post"})
    return nodes


def build_tool(equipment_id: str, kind: str, x: float, band: int):
    root = empty(f"tool.{equipment_id}", (x, -BAND_Z[band], TOOL_Y))
    parts = []
    for part, (size, gltf_loc) in TOOL_SHAPES[kind].items():
        # Manifest locations are glTF (x, up, -forward); Blender is (x, forward, up).
        gx, gy, gz = gltf_loc
        loc = (gx, -gz, gy)
        if part == "chamber" and kind in {"litho", "etch"}:
            obj = cylinder(f"tool.{equipment_id}.{part}", size[0] / 2.0, size[2], loc, part, root)
        else:
            obj = cube(f"tool.{equipment_id}.{part}", size, loc, part, root)
        parts.append(
            {
                "node": obj.name,
                "part": part,
                "explode": list(EXPLODE[part]),
                "aabb": world_aabb(obj),
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


def build_templates():
    foup = cube("foup.template", (1.05, 0.95, 1.05), (0.0, 0.0, 0.525), "foup")
    handle = cube("foup.template.handle", (0.55, 0.08, 0.10), (0.0, -0.50, 0.82), "rail", foup)
    foup_parts = [
        {"node": foup.name, "role": "body", "aabb": world_aabb(foup)},
        {"node": handle.name, "role": "handle", "aabb": world_aabb(handle)},
    ]
    # A FOUP holds 25 wafers; the template is drawn 25 times in level 2.
    wafer = cylinder("wafer.template", 0.35, 0.014, (0.0, 0.0, 0.02), "wafer")
    return foup_parts, [
        {"node": wafer.name, "role": "wafer", "aabb": world_aabb(wafer)}
    ]


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
    scene.cycles.samples = 24
    scene.cycles.use_denoising = False
    scene.render.resolution_x = 1600
    scene.render.resolution_y = 900
    scene.render.film_transparent = False
    scene.world = bpy.data.worlds.new("preview.world")
    scene.world.use_nodes = True
    background = scene.world.node_tree.nodes.get("Background")
    if background is not None:
        background.inputs[0].default_value = (0.035, 0.042, 0.055, 1.0)
        background.inputs[1].default_value = 0.7
    return camera


def render(camera, name: str, location, lens: float):
    scene = bpy.context.scene
    camera.location = location
    camera.data.lens = lens
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
        "export_texcoords": False,
        "export_normals": True,
        "export_materials": "EXPORT",
    }
    bpy.ops.export_scene.gltf(**{k: v for k, v in kwargs.items() if k in props})


def main() -> int:
    reset()
    static = build_static()
    tools = [build_tool(*spec) for spec in TOOLS]
    foup_parts, wafer_parts = build_templates()

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
    export(os.path.join(OUT_DIR, GLB_NAME))
    with open(os.path.join(OUT_DIR, MANIFEST_NAME), "w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2, sort_keys=False)
        handle.write("\n")

    camera = build_preview()
    render(camera, "fx-overview.png", (26.0, -30.0, 20.0), 46.0)
    render(camera, "fx-tool-closeup.png", (-17.0, -12.0, 6.5), 62.0)
    print(
        "wrote",
        os.path.join(OUT_DIR, GLB_NAME),
        os.path.join(OUT_DIR, MANIFEST_NAME),
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
