"""Hard-surface part kit for the fab scene.

The method is AnyMachine's: one real part at a time, boxes with the hidden
faces skipped, one-segment chamfers where they matter, separate meshes that
touch the way the real parts do.  It is adapted from
`hardsurface/dugout_blender/props_hand/_common/hs.py` in
https://github.com/apekshik/anymachine (MIT).

One difference matters here.  That kit splits every object into a game mesh and
a bake-only mesh: bolt heads, weld beads, rivets and panel joints under about
8 mm exist only as a normal map.  The fab renderer (`oxide-render`) draws
positions and normals and nothing else — no UVs, no normal maps, one base
colour per mesh node — so anything that is meant to be seen is built as real
geometry here, and there is no bake step.  Keep detail small and cheap instead:
a hex bolt is 12 triangles, a louvre slat is 12, a flange is a ring of quads.

Coordinates are Blender's: X right, Y back (the tool front faces -Y), Z up,
one unit is one metre.  A `Part` accumulates one bmesh and becomes exactly one
exported mesh node, because the scene manifest names one node per semantic
part.
"""

from __future__ import annotations

import math

import bmesh
import bpy
from mathutils import Matrix, Vector

AXIS = {"x": 0, "y": 1, "z": 2}


def _dirs(spec: str) -> list[Vector]:
    """Parse a skip spec such as "-z,+y" into face normals."""
    out = []
    for token in spec.replace(" ", "").split(","):
        if token:
            normal = [0.0, 0.0, 0.0]
            normal[AXIS[token[1]]] = 1.0 if token[0] == "+" else -1.0
            out.append(Vector(normal))
    return out


def _new_edges(verts) -> list:
    edges = set()
    for vert in verts:
        edges.update(vert.link_edges)
    return list(edges)


class Part:
    """One exported mesh node, built from many small pieces."""

    def __init__(self, name: str):
        self.name = name
        self.bm = bmesh.new()
        self.triangles = 0

    # ------------------------------------------------------------------ primitives

    def box(
        self,
        mn,
        mx,
        bevel: float = 0.0,
        skip: str = "",
        segments: int = 2,
        matrix: Matrix | None = None,
    ) -> None:
        """An axis box.  `skip` names hidden faces ("-z,+y") which are not built.

        `bevel` chamfers every edge whose two faces are kept, so a box that sits
        against another skips the chamfer at the joint it buries.  The box is
        built in its own bmesh because a bevel rebuilds the faces around it and
        invalidates any reference taken before the call.
        """
        mn, mx = Vector(mn), Vector(mx)
        size = mx - mn
        if min(size) <= 0.0:
            raise ValueError(f"{self.name}: inverted box {tuple(mn)}..{tuple(mx)}")
        place = Matrix.Translation((mn + mx) * 0.5)
        if matrix is not None:
            place = matrix @ place
        temp = bmesh.new()
        result = bmesh.ops.create_cube(
            temp, size=1.0, matrix=place @ Matrix.Diagonal((*size, 1.0))
        )
        verts = result["verts"]
        temp.normal_update()
        hidden = _dirs(skip)

        def faces_by_skip(bm):
            return [
                face
                for face in bm.faces
                if any(face.normal.dot(normal) > 0.999 for normal in hidden)
            ]

        dead = faces_by_skip(temp) if hidden else []
        if bevel > 0.0:
            edges = [
                edge
                for edge in _new_edges(verts)
                if not any(face in dead for face in edge.link_faces)
            ]
            if edges:
                bmesh.ops.bevel(
                    temp,
                    geom=edges,
                    offset=bevel,
                    offset_type="OFFSET",
                    segments=segments,
                    profile=0.5,
                    affect="EDGES",
                    clamp_overlap=True,
                )
                temp.normal_update()
                dead = faces_by_skip(temp)
        if dead:
            bmesh.ops.delete(temp, geom=dead, context="FACES")
        mesh = bpy.data.meshes.new("_box")
        temp.to_mesh(mesh)
        temp.free()
        self.bm.from_mesh(mesh)
        bpy.data.meshes.remove(mesh)

    def cyl(
        self,
        p0,
        p1,
        radius: float,
        n: int = 20,
        caps: tuple[bool, bool] = (True, True),
    ) -> None:
        """A cylinder from p0 to p1; `caps` says which ends are closed."""
        p0, p1 = Vector(p0), Vector(p1)
        axis = (p1 - p0)
        if axis.length <= 1e-9:
            raise ValueError(f"{self.name}: zero-length cylinder")
        axis.normalize()
        ex = axis.orthogonal().normalized()
        ey = axis.cross(ex)
        a = [
            self.bm.verts.new(
                p0 + ex * (radius * math.cos(2.0 * math.pi * i / n)) + ey * (radius * math.sin(2.0 * math.pi * i / n))
            )
            for i in range(n)
        ]
        b = [
            self.bm.verts.new(
                p1 + ex * (radius * math.cos(2.0 * math.pi * i / n)) + ey * (radius * math.sin(2.0 * math.pi * i / n))
            )
            for i in range(n)
        ]
        for i in range(n):
            j = (i + 1) % n
            self.bm.faces.new((a[i], a[j], b[j], b[i]))
        if caps[0]:
            self.bm.faces.new(list(reversed(a)))
        if caps[1]:
            self.bm.faces.new(b)

    def lathe(
        self,
        profile,
        at=(0.0, 0.0, 0.0),
        n: int = 20,
        cap_top: bool = False,
        cap_bot: bool = False,
        matrix: Matrix | None = None,
    ) -> None:
        """A profile [(r, z), ...] spun about local Z.  r = 0 closes an end."""
        at = Vector(at)
        rings = []
        for radius, z in profile:
            if radius <= 1e-6:
                rings.append([self.bm.verts.new(at + Vector((0.0, 0.0, z)))])
            else:
                rings.append(
                    [
                        self.bm.verts.new(
                            at
                            + Vector(
                                (
                                    radius * math.cos(2.0 * math.pi * (i + 0.5) / n),
                                    radius * math.sin(2.0 * math.pi * (i + 0.5) / n),
                                    z,
                                )
                            )
                        )
                        for i in range(n)
                    ]
                )
        for lower, upper in zip(rings, rings[1:]):
            for i in range(n):
                j = (i + 1) % n
                if len(lower) == 1 and len(upper) == 1:
                    continue
                if len(lower) == 1:
                    self.bm.faces.new((lower[0], upper[j], upper[i]))
                elif len(upper) == 1:
                    self.bm.faces.new((lower[i], lower[j], upper[0]))
                else:
                    self.bm.faces.new((lower[i], lower[j], upper[j], upper[i]))
        if cap_bot and len(rings[0]) > 1:
            self.bm.faces.new(list(reversed(rings[0])))
        if cap_top and len(rings[-1]) > 1:
            self.bm.faces.new(rings[-1])
        if matrix is not None:
            verts = {vert for ring in rings for vert in ring}
            bmesh.ops.transform(self.bm, matrix=matrix, verts=list(verts))

    def flange(
        self,
        center,
        axis,
        r_out: float,
        r_in: float,
        thickness: float,
        bolts: int = 0,
        bolt_r: float = 0.011,
        bolt_h: float = 0.008,
        n: int = 28,
        bolt_phase: float = 0.5,
    ) -> None:
        """A bolted flange ring: a plate with a hole, plus its bolt circle."""
        center = Vector(center)
        axis = Vector(axis).normalized()
        ex = axis.orthogonal().normalized()
        ey = axis.cross(ex)
        z0, z1 = -thickness * 0.5, thickness * 0.5

        def point(radius: float, i: int, z: float) -> Vector:
            a = 2.0 * math.pi * (i + bolt_phase) / n
            return center + ex * (radius * math.cos(a)) + ey * (radius * math.sin(a)) + axis * z

        outer0 = [self.bm.verts.new(point(r_out, i, z0)) for i in range(n)]
        outer1 = [self.bm.verts.new(point(r_out, i, z1)) for i in range(n)]
        inner0 = [self.bm.verts.new(point(r_in, i, z0)) for i in range(n)]
        inner1 = [self.bm.verts.new(point(r_in, i, z1)) for i in range(n)]
        for i in range(n):
            j = (i + 1) % n
            self.bm.faces.new((outer1[i], outer1[j], inner1[j], inner1[i]))
            self.bm.faces.new((outer0[j], outer0[i], inner0[i], inner0[j]))
            self.bm.faces.new((outer0[i], outer0[j], outer1[j], outer1[i]))
            self.bm.faces.new((inner0[j], inner0[i], inner1[i], inner1[j]))
        if bolts:
            radius = (r_out + r_in) * 0.5
            for i in range(bolts):
                angle = 2.0 * math.pi * (i + bolt_phase) / bolts
                base = center + ex * (radius * math.cos(angle)) + ey * (radius * math.sin(angle))
                self.bolt(base - axis * 0.001, axis, bolt_r, bolt_h)

    def bolt(self, base, axis, radius: float = 0.011, height: float = 0.008) -> None:
        """One hex bolt head (12 triangles)."""
        base, axis = Vector(base), Vector(axis)
        self.cyl(base - axis * 0.001, base + axis * height, radius, n=6, caps=(False, True))

    def bolt_row(self, start, axis, normal, count: int, spacing: float, radius: float = 0.009) -> None:
        """A row of bolts along `axis` standing on `normal`."""
        start, axis, normal = Vector(start), Vector(axis).normalized(), Vector(normal).normalized()
        for i in range(count):
            self.bolt(start + axis * (spacing * i), normal, radius)

    def bolt_circle(self, center, axis, radius: float, count: int, bolt_r: float = 0.009) -> None:
        center, axis = Vector(center), Vector(axis).normalized()
        ex = axis.orthogonal().normalized()
        ey = axis.cross(ex)
        for i in range(count):
            angle = 2.0 * math.pi * (i + 0.5) / count
            base = center + ex * (radius * math.cos(angle)) + ey * (radius * math.sin(angle))
            self.bolt(base, axis, bolt_r)

    def prism(self, outline, ex, ey, t0: float, t1: float, at=(0.0, 0.0, 0.0)) -> None:
        """A convex outline (counter-clockwise in ex, ey) extruded along ex x ey."""
        at, ex, ey = Vector(at), Vector(ex).normalized(), Vector(ey).normalized()
        ez = ex.cross(ey).normalized()
        lower = [self.bm.verts.new(at + ex * u + ey * v + ez * t0) for u, v in outline]
        upper = [self.bm.verts.new(at + ex * u + ey * v + ez * t1) for u, v in outline]
        count = len(outline)
        for i in range(count):
            j = (i + 1) % count
            self.bm.faces.new((lower[i], lower[j], upper[j], upper[i]))
        self.bm.faces.new(list(reversed(lower)))
        self.bm.faces.new(upper)

    def tube(self, points, radius: float, n: int = 10) -> None:
        """A closed pipe through a polyline; each joint is mitred to the angle bisector."""
        points = [Vector(p) for p in points]
        if len(points) < 2:
            raise ValueError("a tube needs two points")
        frames = []
        for index, point in enumerate(points):
            if index == 0:
                tangent = (points[1] - point).normalized()
            elif index == len(points) - 1:
                tangent = (point - points[-2]).normalized()
            else:
                tangent = ((point - points[index - 1]).normalized() + (points[index + 1] - point).normalized())
                tangent.normalize()
            ex = tangent.cross(Vector((0.0, 0.0, 1.0)))
            if ex.length < 0.1:
                ex = tangent.cross(Vector((1.0, 0.0, 0.0)))
            ex.normalize()
            frames.append((point, ex, tangent.cross(ex).normalized()))
        rings = []
        for point, ex, ey in frames:
            rings.append(
                [
                    self.bm.verts.new(
                        point
                        + ex * (radius * math.cos(2.0 * math.pi * i / n))
                        + ey * (radius * math.sin(2.0 * math.pi * i / n))
                    )
                    for i in range(n)
                ]
            )
        for lower, upper in zip(rings, rings[1:]):
            for i in range(n):
                j = (i + 1) % n
                self.bm.faces.new((lower[i], lower[j], upper[j], upper[i]))
        self.bm.faces.new(list(reversed(rings[0])))
        self.bm.faces.new(rings[-1])

    def frame(self, center, ex, ey, width: float, height: float, bar: float, depth: float) -> None:
        """A rectangular ring: a door frame, a window bezel, a panel border.

        Built as one watertight ring (outer and inner outlines), not four bars:
        four bars meeting at the corners leave coplanar faces the renderer would
        z-fight over.
        """
        center, ex, ey = Vector(center), Vector(ex).normalized(), Vector(ey).normalized()
        ez = ex.cross(ey).normalized()
        half_w, half_h = width * 0.5, height * 0.5
        inner_w, inner_h = half_w - bar, half_h - bar
        outline = [(-half_w, -half_h), (half_w, -half_h), (half_w, half_h), (-half_w, half_h)]
        hole = [(-inner_w, -inner_h), (inner_w, -inner_h), (inner_w, inner_h), (-inner_w, inner_h)]

        def ring(offset: float):
            return [self.bm.verts.new(center + ex * u + ey * v + ez * offset) for u, v in outline]

        def ring_inner(offset: float):
            return [self.bm.verts.new(center + ex * u + ey * v + ez * offset) for u, v in hole]

        front_outer, back_outer = ring(depth * 0.5), ring(-depth * 0.5)
        front_inner, back_inner = ring_inner(depth * 0.5), ring_inner(-depth * 0.5)
        for i in range(4):
            j = (i + 1) % 4
            self.bm.faces.new((front_outer[i], front_outer[j], front_inner[j], front_inner[i]))
            self.bm.faces.new((back_outer[j], back_outer[i], back_inner[i], back_inner[j]))
            self.bm.faces.new((back_outer[i], back_outer[j], front_outer[j], front_outer[i]))
            self.bm.faces.new((back_inner[j], back_inner[i], front_inner[i], front_inner[j]))

    def louvres(
        self,
        origin,
        ex,
        ey,
        width: float,
        height: float,
        count: int,
        depth: float = 0.035,
        tilt: float = 38.0,
    ) -> None:
        """A louvered vent: `count` slats tilted about the horizontal edge axis."""
        origin, ex, ey = Vector(origin), Vector(ex).normalized(), Vector(ey).normalized()
        normal = ex.cross(ey).normalized()
        pitch = height / count
        angle = math.radians(tilt)
        for i in range(count):
            slot = origin + ey * ((i + 0.5) * pitch)
            along = (ey * math.cos(angle) + normal * math.sin(angle)).normalized()
            half = along * (depth * 0.5)
            a = slot - ex * (width * 0.5) - half
            b = slot + ex * (width * 0.5) + half
            mn = [min(a[k], b[k]) for k in range(3)]
            mx = [max(a[k], b[k]) for k in range(3)]
            self.box(mn, mx)

    def hinge(
        self,
        center,
        axis,
        normal,
        length: float = 0.10,
        knuckle: float = 0.014,
        leaf: float = 0.05,
    ) -> None:
        """A hinge barrel on `axis` with two leaves lying on the surface `normal`."""
        center, axis, normal = Vector(center), Vector(axis).normalized(), Vector(normal).normalized()
        self.cyl(center - axis * (length * 0.5), center + axis * (length * 0.5), knuckle, n=10)
        # Right-handed basis (tangent, normal, axis): a mirrored one would flip
        # every face of the leaves and shade them inside out.
        tangent = normal.cross(axis).normalized()
        basis = Matrix(
            (
                (tangent.x, normal.x, axis.x, 0.0),
                (tangent.y, normal.y, axis.y, 0.0),
                (tangent.z, normal.z, axis.z, 0.0),
                (0.0, 0.0, 0.0, 1.0),
            )
        )
        for sign in (-1.0, 1.0):
            place = Matrix.Translation(center + tangent * (sign * leaf * 0.5)) @ basis
            self.box(
                (-leaf * 0.5, -0.009, -length * 0.45),
                (leaf * 0.5, 0.009, length * 0.45),
                bevel=0.002,
                matrix=place,
            )

    def handle(self, p0, p1, standoff: float, normal, radius: float = 0.012) -> None:
        """A U handle from p0 to p1, standing off the face along `normal`."""
        p0, p1, normal = Vector(p0), Vector(p1), Vector(normal).normalized()
        self.tube([p0, p0 + normal * standoff, p1 + normal * standoff, p1], radius, n=8)

    def plate(self, center, ex, ey, width: float, height: float, proud: float = 0.002) -> None:
        """A thin plate on a face: labels, nameplates, kick plates.

        It starts 1.5 mm inside the face it sits on, so no face is coplanar with
        the surface and the renderer has nothing to z-fight.
        """
        center, ex, ey = Vector(center), Vector(ex).normalized(), Vector(ey).normalized()
        ez = ex.cross(ey).normalized()
        basis = Matrix(
            (
                (ex.x, ey.x, ez.x, 0.0),
                (ex.y, ey.y, ez.y, 0.0),
                (ex.z, ey.z, ez.z, 0.0),
                (0.0, 0.0, 0.0, 1.0),
            )
        )
        self.box(
            (-width * 0.5, -height * 0.5, -0.0015),
            (width * 0.5, height * 0.5, proud),
            bevel=proud * 0.4,
            matrix=Matrix.Translation(center) @ basis,
        )

    # ------------------------------------------------------------------ output

    def finish(self, material, parent=None, smooth_angle: float = 50.0):
        """Write the bmesh into one object, shaded smoothly with edges above
        `smooth_angle` kept sharp (chamfers read as highlights, corners stay
        crisp) and parent it, local coordinates unchanged.

        Coincident vertices are deliberately not welded: the parts of a node are
        separate pieces of metal, and welding their corners would mix face
        windings and smooth-shade the joints the wrong way.
        """
        mesh = bpy.data.meshes.new(self.name)
        self.bm.to_mesh(mesh)
        self.bm.free()
        mesh.materials.append(material)
        mesh.shade_smooth()
        mesh.set_sharp_from_angle(angle=math.radians(smooth_angle))
        obj = bpy.data.objects.new(self.name, mesh)
        bpy.context.collection.objects.link(obj)
        if parent is not None:
            obj.parent = parent
        return obj


def material(name: str, color, metallic: float = 0.1, roughness: float = 0.45):
    """A Principled material; the renderer only reads the base colour factor."""
    existing = bpy.data.materials.get(name)
    if existing is not None:
        return existing
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    bsdf = next((node for node in mat.node_tree.nodes if node.type == "BSDF_PRINCIPLED"), None)
    if bsdf is not None:
        bsdf.inputs["Base Color"].default_value = color
        bsdf.inputs["Metallic"].default_value = metallic
        bsdf.inputs["Roughness"].default_value = roughness
    mat.diffuse_color = color
    return mat
