"""Bake one shared texture set for the fab scene and put every part's UVs in it.

The method is AnyMachine's bake step (bake the wear shader and the small detail
to a texture set; keep one set per asset family).  Here there is one family --
the factory -- so this module produces one 2048 atlas for the whole scene, with
one rectangular tile per exported mesh node.  The tiles are sized by hand per
part kind, which keeps the texel density predictable and the atlas honest:

    body 320   chamber 256   loadport 256   panel 128   exhaust 128
    floor 640  band 288      rail 288       post 128
    foup 320   handle 64     wafer 96

Three images come out of it, all sampled by the runtime:

    fab-base.png    base colour, sRGB (paint, grime, edge wear, metal show-through)
    fab-normal.png  tangent-space normal (a fine paint orange-peel bump)
    fab-orm.png     R = ambient occlusion, G = roughness, B = metallic

Each object is unwrapped with Smart UV Project, baked on its own, and its UVs are
then scaled and offset into its atlas tile, so the runtime needs one texture set
and no per-node material parameters beyond the existing colour tint.

Baking is Cycles work: a full run is a few minutes.  `--nobake` skips all of it
(the scene still exports, untextured, for geometry checks).
"""

from __future__ import annotations

import math
import os

import bpy
import numpy as np

ATLAS = 2048
GUTTER = 6
MARGIN = 4

TILES = {
    "body": 320,
    "chamber": 256,
    "loadport": 256,
    "panel": 128,
    "exhaust": 128,
    "floor": 640,
    "band": 288,
    "rail": 288,
    "post": 128,
    "foup": 320,
    "handle": 64,
    "wafer": 96,
}

# Metal colour that shows through worn paint, and how strongly each family wears.
WEAR = {
    "body": (0.30, 0.31, 0.33),
    "chamber": (0.42, 0.44, 0.47),
    "loadport": (0.30, 0.31, 0.33),
    "panel": (0.22, 0.23, 0.25),
    "exhaust": (0.40, 0.42, 0.45),
    "floor": (0.16, 0.17, 0.19),
    "band": (0.24, 0.25, 0.27),
    "rail": (0.42, 0.45, 0.50),
    "foup": (0.55, 0.57, 0.60),
    "wafer": (0.45, 0.48, 0.52),
}

# Noise scale per family so grime reads at the same physical size on a 1 m panel
# and on the 44 m floor.
WEAR_SCALE = {
    "floor": 5.0,
    "band": 3.0,
    "rail": 3.5,
    "body": 1.4,
    "chamber": 1.4,
    "loadport": 1.4,
    "exhaust": 1.4,
    "panel": 1.0,
    "foup": 1.0,
    "wafer": 1.0,
}


def family_for(node: str) -> str:
    """The material family a mesh node belongs to (one family, one texture region)."""
    if node.startswith("factory.amhs"):
        return "rail"
    if node.startswith("factory.floor"):
        return "floor"
    if node.startswith("factory.band"):
        return "band"
    if node.startswith("foup.template"):
        return "foup"
    if node.startswith("wafer.template"):
        return "wafer"
    return node.rsplit(".", 1)[-1]


def tile_for(node: str) -> int:
    """The atlas tile size for a node name such as `tool.CLN-01.body`."""
    if node.startswith("foup.template.handle"):
        return TILES["handle"]
    return TILES[family_for(node)]


def pack(tiles: dict[str, int]) -> dict[str, tuple[int, int, int]]:
    """Binary-tree pack square tiles from the bottom-left; returns (x, y, size)."""
    free = [(0, 0, ATLAS, ATLAS)]
    rects: dict[str, tuple[int, int, int]] = {}
    for name in sorted(tiles, key=lambda item: (-tiles[item], item)):
        size = tiles[name]
        need = size + GUTTER
        free.sort(key=lambda rect: (rect[1], rect[0]))
        for index, (x, y, w, h) in enumerate(free):
            if w < need or h < need:
                continue
            rects[name] = (x, y, size)
            free.pop(index)
            if w - need > h - need:
                free.append((x + need, y, w - need, h))
                free.append((x, y + need, size, h - need))
            else:
                free.append((x, y + need, w, h - need))
                free.append((x + need, y, w - need, size))
            free = [rect for rect in free if rect[2] > 0 and rect[3] > 0]
            break
        else:
            raise RuntimeError(f"atlas overflow at {name} ({size}px)")
    return rects


def _wear_graph(nodes, links, color, metallic: float, roughness: float, metal_color, scale: float = 1.0):
    """Paint variation, edge wear, grime and a micro bump; returns the sockets."""

    def noise(size: float, detail: float, coordinate):
        node = nodes.new("ShaderNodeTexNoise")
        node.inputs["Scale"].default_value = size * scale
        node.inputs["Detail"].default_value = detail
        links.new(coordinate, node.inputs["Vector"])
        return node

    coordinates = nodes.new("ShaderNodeTexCoord")
    geometry = nodes.new("ShaderNodeNewGeometry")

    paint = noise(4.5, 4.0, coordinates.outputs["Object"])
    paint_ramp = nodes.new("ShaderNodeValToRGB")
    paint_ramp.color_ramp.elements[0].position = 0.32
    paint_ramp.color_ramp.elements[0].color = (0.82, 0.82, 0.82, 1.0)
    paint_ramp.color_ramp.elements[1].position = 0.68
    paint_ramp.color_ramp.elements[1].color = (1.0, 1.0, 1.0, 1.0)
    links.new(paint.outputs["Fac"], paint_ramp.inputs["Fac"])
    paint_mix = nodes.new("ShaderNodeMix")
    paint_mix.data_type = "RGBA"
    paint_mix.blend_type = "MULTIPLY"
    paint_mix.inputs[0].default_value = 0.55
    paint_mix.inputs[6].default_value = tuple(color)
    links.new(paint_ramp.outputs["Color"], paint_mix.inputs[7])

    edge = nodes.new("ShaderNodeMapRange")
    edge.inputs["From Min"].default_value = 0.58
    edge.inputs["From Max"].default_value = 0.66
    links.new(geometry.outputs["Pointiness"], edge.inputs["Value"])

    grime = noise(11.0, 5.0, coordinates.outputs["Object"])
    grime_ramp = nodes.new("ShaderNodeValToRGB")
    grime_ramp.color_ramp.elements[0].position = 0.40
    grime_ramp.color_ramp.elements[0].color = (0.0, 0.0, 0.0, 1.0)
    grime_ramp.color_ramp.elements[1].position = 0.78
    grime_ramp.color_ramp.elements[1].color = (1.0, 1.0, 1.0, 1.0)
    links.new(grime.outputs["Fac"], grime_ramp.inputs["Fac"])
    grime_mix = nodes.new("ShaderNodeMix")
    grime_mix.data_type = "RGBA"
    grime_mix.blend_type = "MULTIPLY"
    grime_mix.inputs[7].default_value = (0.55, 0.55, 0.56, 1.0)
    grime_amount = nodes.new("ShaderNodeMath")
    grime_amount.operation = "MULTIPLY"
    grime_amount.inputs[1].default_value = 0.5
    links.new(grime_ramp.outputs["Color"], grime_amount.inputs[0])
    links.new(paint_mix.outputs[2], grime_mix.inputs[6])
    links.new(grime_amount.outputs["Value"], grime_mix.inputs[0])

    wear_mix = nodes.new("ShaderNodeMix")
    wear_mix.data_type = "RGBA"
    wear_mix.inputs[7].default_value = (*metal_color, 1.0)
    links.new(grime_mix.outputs[2], wear_mix.inputs[6])
    links.new(edge.outputs["Result"], wear_mix.inputs[0])

    rough = nodes.new("ShaderNodeMapRange")
    rough.inputs["To Min"].default_value = max(0.05, roughness - 0.12)
    rough.inputs["To Max"].default_value = min(0.95, roughness + 0.18)
    links.new(edge.outputs["Result"], rough.inputs["Value"])

    metal = nodes.new("ShaderNodeMapRange")
    metal.inputs["To Min"].default_value = metallic
    metal.inputs["To Max"].default_value = min(1.0, metallic + 0.15)
    links.new(edge.outputs["Result"], metal.inputs["Value"])

    micro = noise(190.0, 2.0, coordinates.outputs["Object"])
    bump = nodes.new("ShaderNodeBump")
    bump.inputs["Strength"].default_value = 0.10
    bump.inputs["Distance"].default_value = 0.01
    links.new(micro.outputs["Fac"], bump.inputs["Height"])

    return {
        "base": wear_mix.outputs["Result"],
        "rough": rough.outputs["Result"],
        "metal": metal.outputs["Result"],
        "normal": bump.outputs["Normal"],
    }


def _material(material_name: str, family: str, color, metallic: float, roughness: float):
    material = bpy.data.materials.new(material_name)
    material.use_nodes = True
    nodes = material.node_tree.nodes
    links = material.node_tree.links
    for node in list(nodes):
        nodes.remove(node)
    output = nodes.new("ShaderNodeOutputMaterial")
    bsdf = nodes.new("ShaderNodeBsdfPrincipled")
    links.new(bsdf.outputs["BSDF"], output.inputs["Surface"])
    sockets = _wear_graph(
        nodes, links, color, metallic, roughness, WEAR[family], WEAR_SCALE[family]
    )
    links.new(sockets["base"], bsdf.inputs["Base Color"])
    links.new(sockets["rough"], bsdf.inputs["Roughness"])
    links.new(sockets["metal"], bsdf.inputs["Metallic"])
    links.new(sockets["normal"], bsdf.inputs["Normal"])
    return material, sockets


def _bake_target(material, image):
    node = material.node_tree.nodes.new("ShaderNodeTexImage")
    node.image = image
    node.select = True
    material.node_tree.nodes.active = node
    return node


def _bake(objects, active, material, size, bake_type, samples, pass_filter=None):
    # Bake targets are always Non-Color: an sRGB target makes Cycles write
    # sRGB-coded values into the float buffer, and the base colour would be
    # encoded twice on the way to the PNG.
    image = bpy.data.images.new(f"bake_{active.name}", size, size, alpha=True, float_buffer=True)
    image.colorspace_settings.name = "Non-Color"
    _bake_target(material, image)
    scene = bpy.context.scene
    scene.cycles.samples = samples
    scene.render.bake.margin = MARGIN
    scene.render.bake.use_clear = True
    scene.render.bake.use_selected_to_active = False
    for other in objects:
        other.hide_render = other != active
    for other in objects:
        other.select_set(False)
    active.select_set(True)
    bpy.context.view_layer.objects.active = active
    if bake_type == "DIFFUSE":
        bpy.ops.object.bake(type="DIFFUSE", pass_filter=pass_filter or {"COLOR"})
    else:
        bpy.ops.object.bake(type=bake_type)
    active.select_set(False)
    pixels = np.array(image.pixels[:], dtype=np.float32).reshape(size, size, 4)
    return image, pixels


def _orm_material(color, metallic, roughness, metal_color, scale: float = 1.0):
    """Emit (0, roughness, metallic); the AO channel is packed from its own bake."""
    material = bpy.data.materials.new("orm_bake")
    material.use_nodes = True
    nodes = material.node_tree.nodes
    links = material.node_tree.links
    for node in list(nodes):
        nodes.remove(node)
    output = nodes.new("ShaderNodeOutputMaterial")
    emission = nodes.new("ShaderNodeEmission")
    links.new(emission.outputs["Emission"], output.inputs["Surface"])
    sockets = _wear_graph(nodes, links, color, metallic, roughness, metal_color, scale)
    combine = nodes.new("ShaderNodeCombineColor")
    links.new(sockets["rough"], combine.inputs["Green"])
    links.new(sockets["metal"], combine.inputs["Blue"])
    links.new(combine.outputs["Color"], emission.inputs["Color"])
    return material


def _unwrap(obj) -> None:
    if bpy.context.object is not None and bpy.context.object.mode != "OBJECT":
        bpy.ops.object.mode_set(mode="OBJECT")
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(angle_limit=math.radians(66), island_margin=0.02)
    bpy.ops.object.mode_set(mode="OBJECT")


def _remap_uvs(obj, rect, atlas: int) -> None:
    x, y, size = rect
    uvs = obj.data.uv_layers.active.data
    pad = 2.0 / atlas
    scale = (size / atlas) - 2.0 * pad
    for loop in uvs:
        loop.uv = (x / atlas + pad + loop.uv[0] * scale, y / atlas + pad + loop.uv[1] * scale)


def _write_atlas(name: str, atlas_pixels, path: str, colorspace: str):
    """Write an 8-bit PNG; the bake buffers are linear floats."""
    pixels = np.clip(atlas_pixels[..., :3], 0.0, 1.0)
    image = bpy.data.images.new(name, ATLAS, ATLAS, alpha=False, float_buffer=False)
    image.colorspace_settings.name = colorspace
    rgba = np.ones((ATLAS, ATLAS, 4), dtype=np.float32)
    rgba[..., :3] = pixels
    image.pixels.foreach_set(np.ascontiguousarray(rgba).reshape(-1))
    image.filepath_raw = path
    image.file_format = "PNG"
    image.save()
    return image


def _runtime_material(name: str, base, normal, orm):
    material = bpy.data.materials.new(name)
    material.use_nodes = True
    nodes = material.node_tree.nodes
    links = material.node_tree.links
    for node in list(nodes):
        nodes.remove(node)
    output = nodes.new("ShaderNodeOutputMaterial")
    bsdf = nodes.new("ShaderNodeBsdfPrincipled")
    links.new(bsdf.outputs["BSDF"], output.inputs["Surface"])

    def image_node(image, colorspace):
        node = nodes.new("ShaderNodeTexImage")
        node.image = image
        node.image.colorspace_settings.name = colorspace
        return node

    base_node = image_node(base, "sRGB")
    normal_node = image_node(normal, "Non-Color")
    orm_node = image_node(orm, "Non-Color")
    links.new(base_node.outputs["Color"], bsdf.inputs["Base Color"])
    normal_map = nodes.new("ShaderNodeNormalMap")
    links.new(normal_node.outputs["Color"], normal_map.inputs["Color"])
    links.new(normal_map.outputs["Normal"], bsdf.inputs["Normal"])
    split = nodes.new("ShaderNodeSeparateColor")
    links.new(orm_node.outputs["Color"], split.inputs["Color"])
    links.new(split.outputs["Green"], bsdf.inputs["Roughness"])
    links.new(split.outputs["Blue"], bsdf.inputs["Metallic"])

    # The exporter writes an occlusionTexture only for a glTF settings group
    # with an input socket called Occlusion.
    settings = bpy.data.node_groups.get("glTF Material Output")
    if settings is None:
        settings = bpy.data.node_groups.new("glTF Material Output", "ShaderNodeTree")
        settings.interface.new_socket(name="Occlusion", in_out="INPUT", socket_type="NodeSocketFloat")
    group = nodes.new("ShaderNodeGroup")
    group.node_tree = settings
    links.new(split.outputs["Red"], group.inputs["Occlusion"])
    return material


def bake_scene(objects, colors, out_dir: str) -> dict:
    """Bake one atlas set for `objects` and assign the texture materials.

    `colors` maps a material family (body, chamber, ...) to (colour, metallic).
    Returns the three atlas images keyed by name.
    """
    tiles = {obj.name: tile_for(obj.name) for obj in objects}
    rects = pack(tiles)
    atlases = {
        "base": np.zeros((ATLAS, ATLAS, 4), dtype=np.float32),
        "normal": np.zeros((ATLAS, ATLAS, 4), dtype=np.float32),
        "orm": np.zeros((ATLAS, ATLAS, 4), dtype=np.float32),
    }
    atlases["base"][..., 3] = 1.0
    atlases["normal"][..., 2] = 1.0
    atlases["normal"][..., 3] = 1.0
    atlases["orm"][..., 3] = 1.0

    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.use_adaptive_sampling = False
    scene.render.bake.use_selected_to_active = False

    for index, obj in enumerate(objects):
        family = family_for(obj.name)
        color, metallic = colors[family]
        size = tiles[obj.name]
        _unwrap(obj)
        material, _ = _material(f"bake_{obj.name}", family, color, metallic, 0.45)
        obj.data.materials.clear()
        obj.data.materials.append(material)
        base_img, base_px = _bake(objects, obj, material, size, "DIFFUSE", 1)
        normal_img, normal_px = _bake(objects, obj, material, size, "NORMAL", 1)
        ao_img, ao_px = _bake(objects, obj, material, size, "AO", 32)
        orm_material = _orm_material(color, metallic, 0.45, WEAR[family], WEAR_SCALE[family])
        obj.data.materials.clear()
        obj.data.materials.append(orm_material)
        orm_img, orm_px = _bake(objects, obj, orm_material, size, "EMIT", 1)
        orm_px[..., 0] = ao_px[..., 0]

        x, y, _ = rects[obj.name]
        atlases["base"][y : y + size, x : x + size] = base_px
        atlases["normal"][y : y + size, x : x + size] = normal_px
        atlases["orm"][y : y + size, x : x + size] = orm_px
        _remap_uvs(obj, rects[obj.name], ATLAS)

        for image in (base_img, normal_img, ao_img, orm_img):
            bpy.data.images.remove(image)
        bpy.data.materials.remove(orm_material)
        bpy.data.materials.remove(material)
        print(f"  baked {obj.name:<28} {size:>3}px  [{index + 1}/{len(objects)}]")

    paths = {
        "base": os.path.join(out_dir, "fab-base.png"),
        "normal": os.path.join(out_dir, "fab-normal.png"),
        "orm": os.path.join(out_dir, "fab-orm.png"),
    }
    images = {
        "base": _write_atlas("fab-base", atlases["base"], paths["base"], "sRGB"),
        "normal": _write_atlas("fab-normal", atlases["normal"], paths["normal"], "Non-Color"),
        "orm": _write_atlas("fab-orm", atlases["orm"], paths["orm"], "Non-Color"),
    }

    by_family: dict[str, list] = {}
    for obj in objects:
        by_family.setdefault(family_for(obj.name), []).append(obj)
    for family, group in sorted(by_family.items()):
        material = _runtime_material(f"fab.{family}", images["base"], images["normal"], images["orm"])
        for obj in group:
            obj.data.materials.clear()
            obj.data.materials.append(material)
            obj.hide_render = False
    return images
