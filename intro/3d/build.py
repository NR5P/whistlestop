"""Build the 3D "Runaway" scene in Blender and save runaway.blend.

  Blender -b --factory-startup -P build.py -- [--preview]

Everything is procedural: winding mountain track (left/right S-curves,
up/down rollercoaster grade, three jumps off the rails, two trestles, a
broken trestle launch), terrain, pines, a 4-6-0 steam engine with tender
and two coaches, volumetric smoke, sparks, storm lighting and a multi-shot
chase camera. Timing comes from ../runaway-timing.json (music_metal.py).
"""
import bisect
import json
import math
import os
import random
import sys

import bmesh
import bpy
from mathutils import Matrix, Quaternion, Vector, noise
from mathutils.kdtree import KDTree

HERE = os.path.dirname(os.path.abspath(__file__))
TM = json.load(open(os.path.join(HERE, "..", "runaway-timing.json")))
ARGS = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
PREVIEW = "--preview" in ARGS
FPS = 30
DUR, T0, T_HIT, T_LAUNCH = TM["dur"], TM["t0"], TM["t_hit"], TM["t_launch"]
BARS, LANDINGS, LIGHTNING = TM["bars"], TM["landings"], TM["lightning"]
NFR = int(round(DUR * FPS))
rng = random.Random(7)


def clamp(x, a=0.0, b=1.0):
    return max(a, min(b, x))


def smooth(x):
    x = clamp(x)
    return x * x * (3 - 2 * x)


def ease_out(x):
    x = clamp(x)
    return 1 - (1 - x) ** 3


def lerp(a, b, u):
    return a + (b - a) * u


# ====================================================================== motion
VS = 0.19  # m/s per bpm


def bpm_at(t):
    for i in range(len(BARS) - 1):
        if t < BARS[i + 1]:
            return 240 / (BARS[i + 1] - BARS[i])
    return TM["bpm1"]


def speed(t):
    cruise = 0.7 * VS * TM["bpm0"]
    if t < T0:
        v = cruise
    else:
        v = lerp(cruise, VS * bpm_at(min(t, T_HIT - 0.01)), ease_out((t - T0) / 1.2))
    if t > T_LAUNCH:
        v *= 1 + 2.2 * clamp((t - T_LAUNCH) / 1.0) ** 2
    return v


DT = 1 / 600
S_TAB = [0.0]
for i in range(int((DUR + 2) / DT)):
    S_TAB.append(S_TAB[-1] + speed(i * DT) * DT)


def S(t):
    i = clamp(t / DT, 0, len(S_TAB) - 2)
    j = int(i)
    return lerp(S_TAB[j], S_TAB[j + 1], i - j)


def t_at_s(s):
    return bisect.bisect_left(S_TAB, s) * DT


# ====================================================================== track
S_DROP = S(T0) - 6
S_END = S(T_LAUNCH)
AIR = 0.85
DS = 0.25
S_MIN, S_MAX = -250.0, S(DUR + 1) + 200


def curv(s):
    env = smooth((s - S_DROP) / 30) * (1 - smooth((s - (S_END - 95)) / 40))
    k = 0.0125 * math.cos((s - S_DROP) / 50) + 0.0065 * math.cos((s - S_DROP) / 21 + 1.3)
    return env * k + 0.0015 * math.cos(s / 80) * (s < S_DROP)


def grade(s):
    if s < S_DROP:
        return 0.0
    return -(0.17 + 0.17 * math.sin((s - S_DROP) / 23)) * smooth((s - S_DROP) / 22)


NS = int((S_MAX - S_MIN) / DS) + 1
PX, PY, PTH, PZ0 = [], [], [], []
x = y = th = z = 0.0
# integrate from s=0 forward and backward so s=0 sits at the origin
fwd = []
s = 0.0
while s <= S_MAX:
    fwd.append((x, y, th, z))
    x += math.cos(th) * DS
    y += math.sin(th) * DS
    th += curv(s) * DS
    z += grade(s) * DS
    s += DS
bwd = []
x = y = th = z = 0.0
s = 0.0
while s > S_MIN:
    s -= DS
    th -= curv(s) * DS
    x -= math.cos(th) * DS
    y -= math.sin(th) * DS
    bwd.append((x, y, th, z))
TAB = list(reversed(bwd)) + fwd
TAB_S0 = -len(bwd) * DS


def tab(s, k):
    i = clamp((s - TAB_S0) / DS, 0, len(TAB) - 2)
    j = int(i)
    return lerp(TAB[j][k], TAB[j + 1][k], i - j)


def heading(s):
    return tab(s, 2)


HUMP_H, HUMP_W = 4.8, 9.0
JUMPS = []
_humps = []
for tl in LANDINGS:
    sa, sl = S(tl - AIR), S(tl)
    _humps.append((sa + 0.25 * HUMP_W, sa, sl))


def z_raw(s, humps):
    z = tab(s, 3)
    for sc, sa, sl in humps:
        z += HUMP_H * math.exp(-((s - sc) / HUMP_W) ** 2)
        z -= 6.0 * math.exp(-((s - (sa + 0.55 * (sl - sa))) / (0.3 * (sl - sa))) ** 2)
    if s > S_END - 30:
        u = clamp((s - (S_END - 30)) / 30)
        z += 8.0 * u * u
    return z


for sc, sa, sl in _humps:
    za = z_raw(sa, _humps)
    ka = (z_raw(sa + 0.1, _humps) - z_raw(sa - 0.1, _humps)) / 0.2
    D = sl - sa
    k = (z_raw(sl, _humps) - za - ka * D) / (D * D)
    JUMPS.append((sc, sa, sl, za, ka, k))


def z_track(s):
    return z_raw(s, _humps)


Z_END = z_track(S_END)
K_SLOPE_END = (z_track(S_END) - z_track(S_END - 0.2)) / 0.2
K_END = -0.0012


def z_car(s):
    if s > S_END:
        d = s - S_END
        return Z_END + K_SLOPE_END * d + K_END * d * d
    z = z_track(s)
    for sc, sa, sl, za, ka, k in JUMPS:
        if sa <= s <= sl:
            d = s - sa
            z = max(z, za + ka * d + k * d * d)
    return z


TRESTLES = [(JUMPS[0][2] + 35, JUMPS[0][2] + 95), (S_END - 85, S_END)]


def canyon(s):
    c = 0.0
    for a, b in TRESTLES:
        c = max(c, smooth((s - a) / 6) * (1 - smooth((s - b) / 6)) if b != S_END else smooth((s - a) / 6))
    return c


def track_point(s, z=None):
    return Vector((tab(s, 0), tab(s, 1), z_track(s) if z is None else z))


def fwd_vec(s):
    th = heading(s)
    return Vector((math.cos(th), math.sin(th), 0))


def left_vec(s):
    th = heading(s)
    return Vector((-math.sin(th), math.cos(th), 0))


# ====================================================================== helpers
def mat_principled(name, color, metal=0.0, rough=0.5, emit=None, strength=0.0, coat=0.0):
    m = bpy.data.materials.new(name)
    pass
    p = m.node_tree.nodes["Principled BSDF"]
    p.inputs["Base Color"].default_value = (*color, 1)
    p.inputs["Metallic"].default_value = metal
    p.inputs["Roughness"].default_value = rough
    if coat:
        p.inputs["Coat Weight"].default_value = coat
        p.inputs["Coat Roughness"].default_value = 0.08
    if emit:
        p.inputs["Emission Color"].default_value = (*emit, 1)
        p.inputs["Emission Strength"].default_value = strength
    return m


def mat_decal(name, img, metal=0.85):
    m = bpy.data.materials.new(name)
    pass
    nt = m.node_tree
    p = nt.nodes["Principled BSDF"]
    tex = nt.nodes.new("ShaderNodeTexImage")
    tex.image = bpy.data.images.load(os.path.join(HERE, "tex", img))
    tex.interpolation = "Cubic"
    nt.links.new(tex.outputs["Color"], p.inputs["Base Color"])
    nt.links.new(tex.outputs["Alpha"], p.inputs["Alpha"])
    p.inputs["Metallic"].default_value = metal
    p.inputs["Roughness"].default_value = 0.3
    p.inputs["Emission Color"].default_value = (1.0, 0.62, 0.22, 1)
    nt.links.new(tex.outputs["Alpha"], p.inputs["Emission Strength"])
    m.surface_render_method = "DITHERED"
    return m


def new_obj(name, bm, mat, parent=None):
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    me.materials.append(mat)
    for poly in me.polygons:
        poly.use_smooth = True
    ob = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(ob)
    if parent:
        ob.parent = parent
    return ob


class Parts:
    """Accumulate primitives per material into one bmesh each."""

    def __init__(self, prefix):
        self.prefix = prefix
        self.bms = {}

    def bm(self, mat):
        if mat.name not in self.bms:
            self.bms[mat.name] = (bmesh.new(), mat)
        return self.bms[mat.name][0]

    def box(self, mat, x0, x1, y0, y1, z0, z1):
        M = Matrix.Translation(((x0 + x1) / 2, (y0 + y1) / 2, (z0 + z1) / 2)) @ Matrix.Diagonal((x1 - x0, y1 - y0, z1 - z0, 1))
        bmesh.ops.create_cube(self.bm(mat), size=1.0, matrix=M)

    def cyl(self, mat, p, axis, r1, r2, length, seg=32):
        rot = Vector((0, 0, 1)).rotation_difference(Vector(axis).normalized()).to_matrix().to_4x4()
        M = Matrix.Translation(p) @ rot
        bmesh.ops.create_cone(self.bm(mat), cap_ends=True, cap_tris=False, segments=seg, radius1=r1, radius2=r2, depth=length, matrix=M)

    def sphere(self, mat, p, r, sz=(1, 1, 1), seg=24):
        M = Matrix.Translation(p) @ Matrix.Diagonal((*sz, 1))
        bmesh.ops.create_uvsphere(self.bm(mat), u_segments=seg, v_segments=seg // 2, radius=r, matrix=M)

    def poly(self, mat, verts, faces):
        b = self.bm(mat)
        vs = [b.verts.new(v) for v in verts]
        for f in faces:
            b.faces.new([vs[i] for i in f])

    def build(self, parent):
        obs = []
        for name, (b, mat) in self.bms.items():
            obs.append(new_obj(f"{self.prefix}_{name}", b, mat, parent))
        return obs


def decal(name, mat, parent, x0, x1, z0, z1, y, side):
    """Vertical plane on a car side (side=+1 left, -1 right), text reads correctly."""
    yy = y * side
    verts = [(x0, yy, z0), (x1, yy, z0), (x1, yy, z1), (x0, yy, z1)]
    me = bpy.data.meshes.new(name)
    me.from_pydata(verts, [], [(0, 1, 2, 3) if side < 0 else (3, 2, 1, 0)])
    uv = me.uv_layers.new()
    us = [0, 1, 1, 0] if side < 0 else [1, 0, 0, 1]
    vs = [0, 0, 1, 1]
    poly = me.polygons[0]
    for li, vi in zip(poly.loop_indices, poly.vertices):
        uv.data[li].uv = (us[vi], vs[vi])
    me.materials.append(mat)
    ob = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(ob)
    ob.parent = parent
    return ob


def front_decal(name, mat, parent, x, y0, y1, z0, z1):
    verts = [(x, y0, z0), (x, y1, z0), (x, y1, z1), (x, y0, z1)]
    me = bpy.data.meshes.new(name)
    me.from_pydata(verts, [], [(0, 1, 2, 3)])
    uv = me.uv_layers.new()
    poly = me.polygons[0]
    for li, vi in zip(poly.loop_indices, poly.vertices):
        uv.data[li].uv = ([0, 1, 1, 0][vi], [0, 0, 1, 1][vi])
    me.materials.append(mat)
    ob = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(ob)
    ob.parent = parent
    return ob


def empty(name):
    e = bpy.data.objects.new(name, None)
    bpy.context.scene.collection.objects.link(e)
    e.rotation_mode = "QUATERNION"
    return e


# ====================================================================== scene reset
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.render.fps = FPS
scene.frame_start, scene.frame_end = 1, NFR
bpy.context.preferences.edit.keyframe_new_interpolation_type = "LINEAR"

# ====================================================================== materials
M = {
    "paint": mat_principled("paint", (0.012, 0.013, 0.016), 0.3, 0.32, coat=0.6),
    "iron": mat_principled("iron", (0.03, 0.03, 0.033), 0.85, 0.42),
    "steel": mat_principled("steel", (0.55, 0.56, 0.6), 1.0, 0.22),
    "brass": mat_principled("brass", (0.95, 0.6, 0.2), 1.0, 0.25),
    "red": mat_principled("red", (0.22, 0.02, 0.014), 0.1, 0.3, coat=0.7),
    "maroon": mat_principled("maroon", (0.1, 0.018, 0.012), 0.1, 0.32, coat=0.7),
    "wheelred": mat_principled("wheelred", (0.25, 0.03, 0.02), 0.2, 0.45),
    "roof": mat_principled("roof", (0.02, 0.02, 0.022), 0.2, 0.5),
    "window": mat_principled("window", (0.1, 0.05, 0.02), 0, 0.2, (1.0, 0.6, 0.28), 2.5),
    "lens": mat_principled("lens", (1, 1, 1), 0, 0.1, (1.0, 0.93, 0.78), 120.0),
    "fire": mat_principled("fire", (0.1, 0.02, 0), 0, 0.5, (1.0, 0.33, 0.04), 40.0),
    "coal": mat_principled("coal", (0.01, 0.01, 0.01), 0.2, 0.35),
    "wood": mat_principled("wood", (0.05, 0.032, 0.02), 0, 0.75),
    "sleeper": mat_principled("sleeper", (0.035, 0.025, 0.018), 0, 0.85),
    "ballast": mat_principled("ballast", (0.04, 0.038, 0.036), 0, 0.9),
    "tree": mat_principled("tree", (0.012, 0.024, 0.016), 0, 0.7),
    "trunk": mat_principled("trunk", (0.02, 0.012, 0.008), 0, 0.9),
    "spark": mat_principled("spark", (1, 0.5, 0.1), 0, 0.5, (1.0, 0.55, 0.15), 60.0),
    "bolt": mat_principled("bolt", (1, 1, 1), 0, 0.5, (0.75, 0.82, 1.0), 400.0),
}
M["herald"] = mat_decal("herald", "herald.png")
M["tender"] = mat_decal("tenderlbl", "tender.png")
M["coach1"] = mat_decal("coach1lbl", "coach1.png")
M["coach2"] = mat_decal("coach2lbl", "coach2.png")
sign_mat = mat_decal("signmat", "sign.png", metal=0.0)


def terrain_material():
    m = bpy.data.materials.new("terrain")
    pass
    nt = m.node_tree
    N = nt.nodes
    p = N["Principled BSDF"]
    tc = N.new("ShaderNodeTexCoord")
    n1 = N.new("ShaderNodeTexNoise")
    n1.inputs["Scale"].default_value = 0.035
    n1.inputs["Detail"].default_value = 10
    n2 = N.new("ShaderNodeTexNoise")
    n2.inputs["Scale"].default_value = 1.6
    n2.inputs["Detail"].default_value = 8
    nt.links.new(tc.outputs["Object"], n1.inputs["Vector"])
    nt.links.new(tc.outputs["Object"], n2.inputs["Vector"])
    ramp = N.new("ShaderNodeValToRGB")
    ramp.color_ramp.elements[0].color = (0.018, 0.017, 0.016, 1)
    ramp.color_ramp.elements[1].color = (0.07, 0.065, 0.058, 1)
    nt.links.new(n1.outputs["Factor"], ramp.inputs["Factor"])
    # snow on flat, high-noise spots
    geo = N.new("ShaderNodeNewGeometry")
    sep = N.new("ShaderNodeSeparateXYZ")
    nt.links.new(geo.outputs["Normal"], sep.inputs["Vector"])
    flat = N.new("ShaderNodeMapRange")
    flat.inputs["From Min"].default_value = 0.88
    flat.inputs["From Max"].default_value = 0.95
    nt.links.new(sep.outputs["Z"], flat.inputs["Value"])
    nmask = N.new("ShaderNodeMapRange")
    nmask.inputs["From Min"].default_value = 0.56
    nmask.inputs["From Max"].default_value = 0.62
    nt.links.new(n1.outputs["Factor"], nmask.inputs["Value"])
    mul = N.new("ShaderNodeMath")
    mul.operation = "MULTIPLY"
    nt.links.new(flat.outputs["Result"], mul.inputs[0])
    nt.links.new(nmask.outputs["Result"], mul.inputs[1])
    mixc = N.new("ShaderNodeMix")
    mixc.data_type = "RGBA"
    nt.links.new(mul.outputs["Value"], mixc.inputs["Factor"])
    nt.links.new(ramp.outputs["Color"], mixc.inputs["A"])
    mixc.inputs["B"].default_value = (0.32, 0.34, 0.38, 1)
    nt.links.new(mixc.outputs["Result"], p.inputs["Base Color"])
    rr = N.new("ShaderNodeMapRange")
    rr.inputs["To Min"].default_value = 0.45
    rr.inputs["To Max"].default_value = 0.85
    nt.links.new(n2.outputs["Factor"], rr.inputs["Value"])
    nt.links.new(rr.outputs["Result"], p.inputs["Roughness"])
    bump = N.new("ShaderNodeBump")
    bump.inputs["Strength"].default_value = 0.5
    nt.links.new(n2.outputs["Factor"], bump.inputs["Height"])
    nt.links.new(bump.outputs["Normal"], p.inputs["Normal"])
    return m


def smoke_material():
    m = bpy.data.materials.new("smoke")
    pass
    nt = m.node_tree
    N = nt.nodes
    N.remove(N["Principled BSDF"])
    out = N["Material Output"]
    vol = N.new("ShaderNodeVolumePrincipled")
    vol.inputs["Color"].default_value = (0.55, 0.56, 0.6, 1)
    vol.inputs["Anisotropy"].default_value = 0.35
    tc = N.new("ShaderNodeTexCoord")
    oi = N.new("ShaderNodeObjectInfo")
    # spherical falloff in object space
    ln = N.new("ShaderNodeVectorMath")
    ln.operation = "LENGTH"
    nt.links.new(tc.outputs["Object"], ln.inputs[0])
    fall = N.new("ShaderNodeMapRange")
    fall.inputs["From Min"].default_value = 1.0
    fall.inputs["From Max"].default_value = 0.35
    nt.links.new(ln.outputs["Value"], fall.inputs["Value"])
    # billowy noise, offset per puff
    add = N.new("ShaderNodeVectorMath")
    add.operation = "ADD"
    nt.links.new(tc.outputs["Object"], add.inputs[0])
    sc = N.new("ShaderNodeVectorMath")
    sc.operation = "SCALE"
    nt.links.new(oi.outputs["Random"], sc.inputs[0])
    sc.inputs["Scale"].default_value = 40
    nt.links.new(sc.outputs["Vector"], add.inputs[1])
    nz = N.new("ShaderNodeTexNoise")
    nz.inputs["Scale"].default_value = 1.8
    nz.inputs["Detail"].default_value = 6
    nz.inputs["Distortion"].default_value = 0.4
    nt.links.new(add.outputs["Vector"], nz.inputs["Vector"])
    nr = N.new("ShaderNodeMapRange")
    nr.inputs["From Min"].default_value = 0.38
    nr.inputs["From Max"].default_value = 0.68
    nt.links.new(nz.outputs["Factor"], nr.inputs["Value"])
    m1 = N.new("ShaderNodeMath")
    m1.operation = "MULTIPLY"
    nt.links.new(fall.outputs["Result"], m1.inputs[0])
    nt.links.new(nr.outputs["Result"], m1.inputs[1])
    m2 = N.new("ShaderNodeMath")
    m2.operation = "MULTIPLY"
    nt.links.new(m1.outputs["Value"], m2.inputs[0])
    nt.links.new(oi.outputs["Alpha"], m2.inputs[1])
    m3 = N.new("ShaderNodeMath")
    m3.operation = "MULTIPLY"
    m3.inputs[1].default_value = 2.6
    nt.links.new(m2.outputs["Value"], m3.inputs[0])
    nt.links.new(m3.outputs["Value"], vol.inputs["Density"])
    nt.links.new(vol.outputs["Volume"], out.inputs["Volume"])
    return m


M["terrain"] = terrain_material()
M["smoke"] = smoke_material()
M["far"] = terrain_material()
M["far"].name = "far"


# ====================================================================== terrain
def path_samples(step=1.0, s0=S_MIN, s1=None):
    s1 = S_END + 400 if s1 is None else s1
    out = []
    s = s0
    while s <= s1:
        out.append(s)
        s += step
    return out


SAMP = path_samples(1.0)
kd = KDTree(len(SAMP))
for i, s in enumerate(SAMP):
    kd.insert((tab(s, 0), tab(s, 1), 0), i)
kd.balance()
Z_SAMP = [z_track(min(s, S_END)) for s in SAMP]


def terrain_h(x, y):
    co, i, dist = kd.find((x, y, 0))
    s = SAMP[i]
    zt = Z_SAMP[i] if s <= S_END else Z_END - 6
    d = (Vector((x, y, 0)) - Vector((tab(s, 0), tab(s, 1), 0))).dot(left_vec(s))
    a = abs(d)
    n1 = noise.fractal(Vector((x * 0.004, y * 0.004, 0.3)), 1.0, 2.0, 5)
    n2 = noise.fractal(Vector((x * 0.03, y * 0.03, 1.7)), 0.8, 2.0, 4)
    n3 = noise.noise(Vector((x * 0.25, y * 0.25, 4.0)))
    base = zt - 0.62
    if a < 3.3:
        h = base
    elif d > 0:  # mountain wall on the left
        e = a - 3.3
        h = base + e * 0.8 + 0.003 * e * e + n2 * 4 * smooth(e / 8) + n1 * 30 * smooth(e / 60)
    else:        # drop to the valley on the right
        e = a - 3.3
        h = base - e * 0.55 + 0.0015 * e * e + n2 * 3.5 * smooth(e / 8) + n1 * 25 * smooth(e / 60)
    h += n3 * 0.5 * smooth((a - 3) / 4)
    c = canyon(s) if s <= S_END else 1.0
    if c > 0:
        hc = zt - 75 + n2 * 10 + n1 * 8
        h = lerp(h, hc, c * (1 - smooth((a - 28) / 30)))
    return h


def build_terrain():
    xs = [tab(s, 0) for s in SAMP]
    ys = [tab(s, 1) for s in SAMP]
    pad = 380
    x0, x1 = min(xs) - pad, max(xs) + pad
    y0, y1 = min(ys) - pad, max(ys) + pad
    step = 6.0 if PREVIEW else 3.5
    nx, ny = int((x1 - x0) / step) + 1, int((y1 - y0) / step) + 1
    verts = []
    for j in range(ny):
        for i in range(nx):
            x, y = x0 + i * step, y0 + j * step
            verts.append((x, y, terrain_h(x, y)))
    faces = []
    for j in range(ny - 1):
        for i in range(nx - 1):
            a = j * nx + i
            faces.append((a, a + 1, a + nx + 1, a + nx))
    me = bpy.data.meshes.new("terrain")
    me.from_pydata(verts, [], faces)
    me.materials.append(M["terrain"])
    for p in me.polygons:
        p.use_smooth = True
    ob = bpy.data.objects.new("terrain", me)
    scene.collection.objects.link(ob)
    return (x0, x1, y0, y1)


BOUNDS = build_terrain()


def build_far_mountains():
    cx = (BOUNDS[0] + BOUNDS[1]) / 2
    cy = (BOUNDS[2] + BOUNDS[3]) / 2
    R0 = max(BOUNDS[1] - BOUNDS[0], BOUNDS[3] - BOUNDS[2]) * 0.55 + 200
    na, nr = 360, 14
    verts = []
    for j in range(nr):
        r = R0 + j * 120
        for i in range(na):
            a = 2 * math.pi * i / na
            x, y = cx + r * math.cos(a), cy + r * math.sin(a)
            ridge = 1 - abs(noise.noise(Vector((math.cos(a) * 3, math.sin(a) * 3, j * 0.05))))
            h = -120 + 380 * smooth(j / 5) * (0.4 + ridge ** 2) + 120 * noise.fractal(Vector((x * 0.003, y * 0.003, 2)), 1, 2, 5)
            verts.append((x, y, h))
    faces = []
    for j in range(nr - 1):
        for i in range(na):
            a, b = j * na + i, j * na + (i + 1) % na
            faces.append((a, b, b + na, a + na))
    me = bpy.data.meshes.new("far")
    me.from_pydata(verts, [], faces)
    me.materials.append(M["far"])
    for p in me.polygons:
        p.use_smooth = True
    ob = bpy.data.objects.new("far", me)
    scene.collection.objects.link(ob)


build_far_mountains()


def build_trees():
    verts, faces = [], []
    tv, tf = [], []
    r = random.Random(11)
    n = 1800 if PREVIEW else 5200
    x0, x1, y0, y1 = BOUNDS
    placed = 0
    tries = 0
    while placed < n and tries < n * 6:
        tries += 1
        x, y = r.uniform(x0 + 30, x1 - 30), r.uniform(y0 + 30, y1 - 30)
        co, i, dist = kd.find((x, y, 0))
        if dist < 7.5 or dist > 330:
            continue
        s = SAMP[i]
        if s > S_END - 90 or canyon(s) > 0.2:
            if dist < 60:
                continue
        h = terrain_h(x, y)
        gx = terrain_h(x + 1.5, y) - terrain_h(x - 1.5, y)
        gy = terrain_h(x, y + 1.5) - terrain_h(x, y - 1.5)
        if math.hypot(gx, gy) / 3 > 1.3:
            continue
        H = r.uniform(7, 19)
        R = H * r.uniform(0.2, 0.28)
        seg = 9
        # trunk
        b = len(tv)
        for k in range(seg):
            a = 2 * math.pi * k / seg
            tv.append((x + 0.25 * math.cos(a), y + 0.25 * math.sin(a), h - 1))
            tv.append((x + 0.18 * math.cos(a), y + 0.18 * math.sin(a), h + H * 0.3))
        for k in range(seg):
            k2 = (k + 1) % seg
            tf.append((b + 2 * k, b + 2 * k2, b + 2 * k2 + 1, b + 2 * k + 1))
        # three tiers of foliage
        for tier in range(5):
            zb = h + H * (0.15 + tier * 0.16)
            zt = zb + H * (0.36 if tier < 4 else 0.3)
            rr = R * (1.05 - tier * 0.19) * r.uniform(0.85, 1.15)
            b = len(verts)
            for k in range(seg):
                a = 2 * math.pi * k / seg + r.uniform(-0.15, 0.15)
                rk = rr * (1.0 if k % 2 == 0 else 0.72)
                verts.append((x + rk * math.cos(a), y + rk * math.sin(a), zb + r.uniform(-0.45, 0.15)))
            verts.append((x, y, zt))
            for k in range(seg):
                faces.append((b + k, b + (k + 1) % seg, b + seg))
            faces.append(tuple(b + k for k in range(seg - 1, -1, -1)))
        placed += 1
    for name, v, f, mat in (("trees", verts, faces, M["tree"]), ("trunks", tv, tf, M["trunk"])):
        me = bpy.data.meshes.new(name)
        me.from_pydata(v, [], f)
        me.materials.append(mat)
        for p in me.polygons:
            p.use_smooth = True
        ob = bpy.data.objects.new(name, me)
        scene.collection.objects.link(ob)


build_trees()


# ====================================================================== track
def build_track():
    rails_v, rails_f = [], []
    sl_v, sl_f = [], []
    bal_v, bal_f = [], []
    s = S_MIN + 20
    prev = None
    profile = [(-0.035, -0.17), (0.035, -0.17), (0.035, 0.0), (-0.035, 0.0)]
    bal_prof = [(-2.6, -0.62), (-1.6, -0.25), (1.6, -0.25), (2.6, -0.62)]
    while s <= S_END:
        p = track_point(s)
        L = left_vec(s)
        ring = []
        for side in (-0.7175, 0.7175):
            for (dy, dz) in profile:
                ring.append(p + L * (side + dy) + Vector((0, 0, dz)))
        base = len(rails_v)
        rails_v.extend(ring)
        if prev is not None:
            for r_ in range(2):
                for k in range(4):
                    a = prev + r_ * 4 + k
                    b2 = prev + r_ * 4 + (k + 1) % 4
                    rails_f.append((a, b2, base + r_ * 4 + (k + 1) % 4, base + r_ * 4 + k))
        if canyon(s) < 0.5:
            bb = len(bal_v)
            for (dy, dz) in bal_prof:
                bal_v.append(p + L * dy + Vector((0, 0, dz)))
            if prev is not None and len(bal_v) >= 8 and bal_ok:
                for k in range(3):
                    bal_f.append((bb - 4 + k, bb - 4 + k + 1, bb + k + 1, bb + k))
            bal_ok = True
        else:
            bal_ok = False
        prev = base
        s += 0.5
    # sleepers every 0.6 m
    s = S_MIN + 20
    while s <= S_END - 1:
        p = track_point(s)
        F, L = fwd_vec(s), left_vec(s)
        broken = s > S_END - 6 and rng.random() < 0.5
        if not broken:
            b = len(sl_v)
            for dz in (-0.35, -0.17):
                for (u, w) in ((-0.13, -1.3), (0.13, -1.3), (0.13, 1.3), (-0.13, 1.3)):
                    sl_v.append(p + F * u + L * w + Vector((0, 0, dz)))
            sl_f += [(b, b + 1, b + 2, b + 3), (b + 7, b + 6, b + 5, b + 4), (b, b + 4, b + 5, b + 1),
                     (b + 1, b + 5, b + 6, b + 2), (b + 2, b + 6, b + 7, b + 3), (b + 3, b + 7, b + 4, b)]
        s += 0.6
    for name, v, f, mat in (("rails", rails_v, rails_f, M["steel"]), ("sleepers", sl_v, sl_f, M["sleeper"]), ("ballast", bal_v, bal_f, M["ballast"])):
        me = bpy.data.meshes.new(name)
        me.from_pydata([tuple(q) for q in v], [], f)
        me.materials.append(mat)
        ob = bpy.data.objects.new(name, me)
        scene.collection.objects.link(ob)


bal_ok = False
build_track()


def build_trestles():
    P = Parts("trestle")
    bm = P.bm(M["wood"])
    for a, b in TRESTLES:
        s = a
        while s <= b:
            p = track_point(s)
            F, L = fwd_vec(s), left_vec(s)
            depth = 90
            for side in (-1, 1):
                top = p + L * (side * 1.4) + Vector((0, 0, -0.4))
                bot = p + L * (side * 7.0) + Vector((0, 0, -depth))
                d = bot - top
                rot = Vector((0, 0, 1)).rotation_difference(d.normalized()).to_matrix().to_4x4()
                Mx = Matrix.Translation((top + bot) / 2) @ rot @ Matrix.Diagonal((0.35, 0.35, d.length, 1))
                bmesh.ops.create_cube(bm, size=1.0, matrix=Mx)
            for lvl in range(0, depth, 9):
                for side in (-1, 1):
                    q = p + Vector((0, 0, -0.5 - lvl))
                    w = 1.4 + (7.0 - 1.4) * lvl / depth
                    Mx = Matrix.Translation(q) @ Matrix.Rotation(heading(s), 4, "Z") @ Matrix.Diagonal((0.25, 2 * w, 0.3, 1))
                    bmesh.ops.create_cube(bm, size=1.0, matrix=Mx)
            # deck beam
            Mx = Matrix.Translation(p + Vector((0, 0, -0.45))) @ Matrix.Rotation(heading(s), 4, "Z") @ Matrix.Diagonal((5.2, 3.2, 0.35, 1))
            bmesh.ops.create_cube(bm, size=1.0, matrix=Mx)
            s += 5.0
    P.build(None)


build_trestles()


def build_sign():
    s = S(1.7) + 30
    p = track_point(s)
    L = left_vec(s)
    th = heading(s)
    base = p + L * 4.6
    e = empty("sign")
    e.location = base
    e.rotation_quaternion = Quaternion((0, 0, 1), th)
    P = Parts("signp")
    P.box(M["wood"], -1.8, -1.6, -0.1, 0.1, -0.6, 3.2)
    P.box(M["wood"], 1.6, 1.8, -0.1, 0.1, -0.6, 3.2)
    P.box(M["wood"], -2.0, 2.0, -0.06, 0.06, 1.9, 3.3)
    P.build(e)
    # face the board toward the track (side -1 = facing -y in its frame)
    decal("signface", sign_mat, e, -1.95, 1.95, 1.95, 3.25, 0.07, -1)
    lt = bpy.data.lights.new("signlamp", "POINT")
    lt.energy = 120
    lt.color = (1.0, 0.7, 0.4)
    lo = bpy.data.objects.new("signlamp", lt)
    scene.collection.objects.link(lo)
    lo.parent = e
    lo.location = (0, -1.2, 3.6)


build_sign()


# ====================================================================== the train
def wheelset(name, parent, x, r, spokes, mat_spoke):
    e = bpy.data.objects.new(name, None)
    scene.collection.objects.link(e)
    e.parent = parent
    e.location = (x, 0, r)
    P = Parts(name)
    for side in (-1, 1):
        y = side * 0.75
        P.cyl(M["iron"], (0, y, 0), (0, 1, 0), r, r, 0.14, 40)
        P.cyl(M["steel"], (0, y + side * 0.02, 0), (0, 1, 0), r * 0.98, r * 0.98, 0.12, 40)
        P.cyl(mat_spoke, (0, y + side * 0.03, 0), (0, 1, 0), r * 0.86, r * 0.86, 0.1, 40)
        for k in range(spokes):
            a = 2 * math.pi * k / spokes
            Mx = Matrix.Translation((0, y + side * 0.07, 0)) @ Matrix.Rotation(a, 4, "Y") @ Matrix.Translation((r * 0.5, 0, 0)) @ Matrix.Diagonal((r * 0.72, 0.06, 0.07, 1))
            bmesh.ops.create_cube(P.bm(M["paint"]), size=1.0, matrix=Mx)
        P.cyl(M["brass"], (0, y + side * 0.1, 0), (0, 1, 0), r * 0.17, r * 0.15, 0.08, 20)
    P.cyl(M["iron"], (0, 0, 0), (0, 1, 0), 0.09, 0.09, 1.5, 12)
    P.build(e)
    return e


def build_loco():
    root = empty("loco")
    P = Parts("loco")
    P.box(M["iron"], -5.6, 5.7, -0.55, 0.55, 0.55, 1.05)                    # frame
    P.box(M["paint"], -3.3, 5.5, -1.45, 1.45, 1.66, 1.72)                   # running board
    P.box(M["brass"], -3.3, 5.5, 1.43, 1.47, 1.6, 1.72)
    P.box(M["brass"], -3.3, 5.5, -1.47, -1.43, 1.6, 1.72)
    P.cyl(M["paint"], (0.55, 0, 2.55), (1, 0, 0), 0.82, 0.82, 7.7, 48)      # boiler
    P.cyl(M["iron"], (5.0, 0, 2.55), (1, 0, 0), 0.86, 0.86, 1.2, 48)        # smokebox
    P.cyl(M["brass"], (5.62, 0, 2.55), (1, 0, 0), 0.82, 0.82, 0.04, 48)
    P.cyl(M["iron"], (5.65, 0, 2.55), (1, 0, 0), 0.74, 0.74, 0.04, 48)
    for bx in (-2.6, -0.9, 0.9, 2.7, 4.4):
        P.cyl(M["brass"], (bx, 0, 2.55), (1, 0, 0), 0.835, 0.835, 0.07, 48)
    # diamond stack
    P.cyl(M["iron"], (4.95, 0, 3.75), (0, 0, 1), 0.27, 0.25, 0.9, 24)
    P.cyl(M["iron"], (4.95, 0, 4.42), (0, 0, 1), 0.25, 0.66, 0.45, 24)
    P.cyl(M["iron"], (4.95, 0, 4.82), (0, 0, 1), 0.66, 0.36, 0.36, 24)
    P.cyl(M["brass"], (4.95, 0, 5.02), (0, 0, 1), 0.38, 0.38, 0.06, 24)
    P.cyl(M["brass"], (4.95, 0, 4.2), (0, 0, 1), 0.28, 0.28, 0.06, 24)
    # domes, bell, whistle
    P.sphere(M["brass"], (1.2, 0, 3.25), 0.46, (1, 1, 0.95))
    P.cyl(M["brass"], (1.2, 0, 3.3), (0, 0, 1), 0.46, 0.46, 0.25, 32)
    P.sphere(M["brass"], (2.75, 0, 3.22), 0.38, (1, 1, 0.9))
    P.cyl(M["brass"], (3.7, 0, 3.55), (0, 0, 1), 0.14, 0.24, 0.32, 20)
    P.cyl(M["brass"], (-3.0, 0, 3.55), (0, 0, 1), 0.06, 0.06, 0.5, 12)
    P.cyl(M["brass"], (-3.0, 0, 3.85), (0, 0, 1), 0.09, 0.09, 0.2, 12)
    # headlamp
    P.box(M["iron"], 5.25, 5.92, -0.36, 0.36, 3.42, 4.02)
    P.box(M["brass"], 5.2, 5.97, -0.38, 0.38, 4.02, 4.07)
    P.cyl(M["lens"], (5.93, 0, 3.72), (1, 0, 0), 0.24, 0.24, 0.03, 32)
    P.cyl(M["brass"], (5.93, 0, 3.72), (1, 0, 0), 0.27, 0.27, 0.02, 32)
    # cab
    P.box(M["red"], -5.8, -3.3, -1.45, 1.45, 1.72, 4.0)
    P.box(M["roof"], -6.15, -3.05, -1.62, 1.62, 4.0, 4.14)
    for side in (-1, 1):
        P.box(M["window"], -5.0, -3.9, side * 1.451 - 0.0005, side * 1.452, 2.9, 3.6)
        P.box(M["brass"], -5.06, -3.84, side * 1.452 - 0.001, side * 1.458, 3.6, 3.65)
    P.box(M["window"], -3.31, -3.29, -1.1, -0.35, 3.0, 3.6)
    P.box(M["window"], -3.31, -3.29, 0.35, 1.1, 3.0, 3.6)
    # firebox glow
    P.box(M["iron"], -5.6, -3.3, -0.62, 0.62, 0.95, 1.66)
    P.box(M["fire"], -5.5, -3.4, -0.63, 0.63, 1.0, 1.08)
    # cylinders
    for side in (-1, 1):
        P.cyl(M["iron"], (4.25, side * 1.2, 1.15), (1, 0, 0), 0.38, 0.38, 1.0, 32)
        P.cyl(M["brass"], (3.75, side * 1.2, 1.15), (1, 0, 0), 0.4, 0.4, 0.06, 32)
        P.cyl(M["brass"], (4.75, side * 1.2, 1.15), (1, 0, 0), 0.4, 0.4, 0.06, 32)
        P.box(M["steel"], 2.4, 3.75, side * 1.2 - 0.05, side * 1.2 + 0.05, 1.29, 1.33)  # guide bar
    # buffer beam + pilot
    P.box(M["red"], 5.55, 5.8, -1.4, 1.4, 1.0, 1.38)
    pv = [(5.6, -1.35, 1.0), (5.6, 1.35, 1.0), (5.6, 1.35, 0.12), (5.6, -1.35, 0.12), (6.95, -0.15, 0.12), (6.95, 0.15, 0.12)]
    P.poly(M["iron"], pv, [(0, 1, 2, 3), (3, 2, 5, 4), (0, 4, 5, 1), (0, 3, 4), (1, 5, 2)])
    for k in range(-5, 6):
        yb = k * 0.24
        P.box(M["steel"], 5.62 + (1 - abs(yb) / 1.35) * 1.2 * 0.9, 5.62 + (1 - abs(yb) / 1.35) * 1.2 * 0.9 + 0.05, yb - 0.025, yb + 0.025, 0.14, 0.95)
    P.build(root)
    front_decal("loco_herald", M["herald"], root, 5.68, -0.6, 0.6, 1.95, 3.15)
    for side in (-1, 1):
        decal(f"loco_herald_{side}", M["herald"], root, -5.15, -3.95, 1.9, 2.75, 1.47, side)
    wheels = []
    for x in (1.8, 0.0, -1.8):
        wheels.append((wheelset(f"drv{x}", root, x, 0.86, 16, M["wheelred"]), 0.86))
    for x in (4.95, 3.65):
        wheels.append((wheelset(f"lead{x}", root, x, 0.45, 10, M["wheelred"]), 0.45))
    # rods
    rods = []
    for side in (-1, 1):
        Pr = Parts(f"crod{side}")
        Pr.box(M["steel"], -1.85, 1.85, -0.04, 0.04, -0.07, 0.07)
        e = bpy.data.objects.new(f"crod{side}", None)
        scene.collection.objects.link(e)
        e.parent = root
        Pr.build(e)
        Pm = Parts(f"mrod{side}")
        Pm.box(M["steel"], 0.0, 3.1, -0.045, 0.045, -0.08, 0.08)
        em = bpy.data.objects.new(f"mrod{side}", None)
        scene.collection.objects.link(em)
        em.parent = root
        em.rotation_mode = "XYZ"
        Pm.build(em)
        Pc = Parts(f"xh{side}")
        Pc.box(M["steel"], -0.18, 0.18, -0.08, 0.08, -0.12, 0.12)
        Pc.cyl(M["steel"], (0.7, 0, 0), (1, 0, 0), 0.05, 0.05, 1.3, 12)
        ec = bpy.data.objects.new(f"xh{side}", None)
        scene.collection.objects.link(ec)
        ec.parent = root
        Pc.build(ec)
        rods.append((side, e, em, ec))
    # lights
    hl = bpy.data.lights.new("headlamp", "SPOT")
    hl.energy = 60000 if not PREVIEW else 60000
    hl.spot_size = math.radians(38)
    hl.spot_blend = 0.6
    hl.color = (1.0, 0.86, 0.62)
    hl.shadow_soft_size = 0.15
    ho = bpy.data.objects.new("headlamp", hl)
    scene.collection.objects.link(ho)
    ho.parent = root
    ho.location = (6.05, 0, 3.72)
    ho.rotation_euler = (0, math.radians(90 + 6), 0)
    beam = bpy.data.materials.new("beam")
    bn = beam.node_tree.nodes
    bn.remove(bn["Principled BSDF"])
    bv = bn.new("ShaderNodeVolumePrincipled")
    bv.inputs["Density"].default_value = 0.035
    bv.inputs["Anisotropy"].default_value = 0.6
    bv.inputs["Color"].default_value = (0.9, 0.85, 0.75, 1)
    beam.node_tree.links.new(bv.outputs["Volume"], bn["Material Output"].inputs["Volume"])
    bb = bmesh.new()
    bmesh.ops.create_cone(bb, cap_ends=True, segments=24, radius1=0.25, radius2=16, depth=55,
                          matrix=Matrix.Translation((6.0 + 27.5, 0, 3.72)) @ Matrix.Rotation(math.radians(90), 4, "Y") @ Matrix.Rotation(math.radians(180), 4, "X"))
    bb.free()  # beam disabled: Eevee fills the bounding box
    fl = bpy.data.lights.new("firebox", "POINT")
    fl.energy = 900
    fl.color = (1.0, 0.4, 0.1)
    fo = bpy.data.objects.new("firebox", fl)
    scene.collection.objects.link(fo)
    fo.parent = root
    fo.location = (-4.5, 0, 0.8)
    cl = bpy.data.lights.new("cablight", "POINT")
    cl.energy = 150
    cl.color = (1.0, 0.55, 0.2)
    co = bpy.data.objects.new("cablight", cl)
    scene.collection.objects.link(co)
    co.parent = root
    co.location = (-4.5, 0, 3.2)
    return root, wheels, rods, fl


def build_tender():
    root = empty("tender")
    P = Parts("tender")
    P.box(M["iron"], -3.5, 3.5, -0.6, 0.6, 0.6, 1.1)
    P.box(M["paint"], -3.4, 3.4, -1.45, 1.45, 1.1, 3.25)
    P.box(M["brass"], -3.42, 3.42, -1.47, 1.47, 3.2, 3.27)
    r = random.Random(5)
    verts, faces = [], []
    nx, ny = 14, 6
    for j in range(ny + 1):
        for i in range(nx + 1):
            xx = -3.2 + 5.0 * i / nx
            yy = -1.35 + 2.7 * j / ny
            hump = 0.5 * math.sin(math.pi * i / nx) * math.sin(math.pi * j / ny)
            verts.append((xx, yy, 3.25 + hump + r.uniform(0, 0.12)))
    for j in range(ny):
        for i in range(nx):
            a = j * (nx + 1) + i
            faces.append((a, a + 1, a + nx + 2, a + nx + 1))
    P.poly(M["coal"], verts, faces)
    P.build(root)
    for side in (-1, 1):
        decal(f"tender_lbl{side}", M["tender"], root, -3.0, 3.0, 1.45, 2.95, 1.47, side)
    wheels = [(wheelset(f"tw{x}", root, x, 0.45, 8, M["iron"]), 0.45) for x in (-2.9, -1.7, 1.7, 2.9)]
    return root, wheels


def build_coach(name, label_mat, idx):
    root = empty(name)
    P = Parts(name)
    P.box(M["iron"], -8.0, 8.0, -0.6, 0.6, 0.6, 1.1)
    P.box(M["maroon"], -8.3, 8.3, -1.45, 1.45, 1.1, 3.55)
    P.box(M["roof"], -8.5, 8.5, -1.58, 1.58, 3.55, 3.7)
    P.box(M["roof"], -7.6, 7.6, -0.95, 0.95, 3.7, 4.1)
    P.box(M["window"], -7.4, 7.4, -0.951, 0.951, 3.78, 3.98)
    for side in (-1, 1):
        P.box(M["brass"], -8.3, 8.3, side * 1.452 - 0.002, side * 1.458, 2.25, 2.29)
        P.box(M["brass"], -8.3, 8.3, side * 1.452 - 0.002, side * 1.458, 1.2, 1.23)
        for k in range(11):
            wx = -7.2 + k * 1.42
            P.box(M["window"], wx, wx + 0.82, side * 1.451 - 0.0005, side * 1.452, 2.45, 3.3)
    P.build(root)
    for side in (-1, 1):
        decal(f"{name}_lbl{side}", label_mat, root, -4.2, 4.2, 1.42, 2.15, 1.47, side)
    lt = bpy.data.lights.new(f"{name}_inside", "POINT")
    lt.energy = 400
    lt.color = (1.0, 0.6, 0.28)
    lo = bpy.data.objects.new(f"{name}_inside", lt)
    scene.collection.objects.link(lo)
    lo.parent = root
    lo.location = (0, 0, 2.8)
    wheels = [(wheelset(f"{name}w{x}", root, x, 0.45, 8, M["iron"]), 0.45) for x in (-6.9, -5.1, 5.1, 6.9)]
    return root, wheels


loco, loco_wheels, rods, firebox = build_loco()
tender, tender_wheels = build_tender()
coach1, coach1_wheels = build_coach("coach1", M["coach1"], 0)
coach2, coach2_wheels = build_coach("coach2", M["coach2"], 1)

# car layout: (object, local x of front pivot, rear pivot, offset of origin behind the pilot tip)
CARS = [
    (loco, 4.3, -1.8, 6.95, loco_wheels),
    (tender, 2.3, -2.3, 6.95 + 10.0, tender_wheels),
    (coach1, 6.0, -6.0, 6.95 + 22.5, coach1_wheels),
    (coach2, 6.0, -6.0, 6.95 + 39.9, coach2_wheels),
]


def lean(s, t):
    v = speed(t)
    k = (curv(s - 4) + curv(s) + curv(s + 4)) / 3
    r = clamp(-0.02 * v * v * k, -0.33, 0.33)
    r += 0.025 * math.sin(t * 9.0 + s * 0.4) * clamp((v - 22) / 10)
    return r


def car_matrix(t, a, b, back):
    sc = S(t) - back
    s1, s2 = sc + a, sc + b
    p1 = Vector((tab(s1, 0), tab(s1, 1), z_car(s1)))
    p2 = Vector((tab(s2, 0), tab(s2, 1), z_car(s2)))
    f = (p1 - p2).normalized()
    L0 = left_vec(sc)
    up = f.cross(L0).normalized()
    L0 = up.cross(f).normalized()
    rho = lean(sc, t)
    air = max(z_car(s1) - z_track(s1), z_car(s2) - z_track(s2)) if s1 < S_END else 5.0
    if air > 0.3:
        rho += 0.12 * math.sin(t * 6 + back) * clamp(air / 3)
    q = Quaternion(f, rho)
    Lr = q @ L0
    upr = q @ up
    loc = p1 - f * a
    yp = -math.copysign(0.75, rho) if abs(rho) > 1e-4 else 0
    loc = loc + (L0 - Lr) * yp
    R = Matrix((f, Lr, upr)).transposed()
    return loc, R.to_quaternion()


# ====================================================================== animate the train
def key_obj(ob, loc, quat, f):
    ob.location = loc
    ob.rotation_quaternion = quat
    ob.keyframe_insert("location", frame=f)
    ob.keyframe_insert("rotation_quaternion", frame=f)


print("animating train ...")
prevq = {}
for fr in range(1, NFR + 1):
    t = (fr - 1) / FPS
    for ob, a, b, back, wheels in CARS:
        loc, q = car_matrix(t, a, b, back)
        pq = prevq.get(ob.name)
        if pq is not None and pq.dot(q) < 0:
            q = -q
        prevq[ob.name] = q
        key_obj(ob, loc, q, fr)
        dist = S(t)
        for w, r in wheels:
            w.rotation_euler = (0, dist / r, 0)
            w.keyframe_insert("rotation_euler", index=1, frame=fr)
    phi = S(t) / 0.86
    for side, crod, mrod, xh in rods:
        ph = phi + (math.pi / 2 if side < 0 else 0)
        rc = 0.33
        cx_, cz_ = rc * math.cos(ph), 0.86 - rc * math.sin(ph)
        crod.location = (cx_, side * 0.98, cz_)
        crod.keyframe_insert("location", frame=fr)
        dz = 1.15 - cz_
        Lr = 3.1
        xh_ = cx_ + math.sqrt(Lr * Lr - dz * dz)
        mrod.location = (cx_, side * 1.08, cz_)
        mrod.rotation_euler = (0, math.atan2(-dz, xh_ - cx_), 0)
        mrod.keyframe_insert("location", frame=fr)
        mrod.keyframe_insert("rotation_euler", index=1, frame=fr)
        xh.location = (xh_, side * 1.12, 1.15)
        xh.keyframe_insert("location", frame=fr)
    firebox.energy = 900 * (0.75 + 0.25 * math.sin(t * 23) * math.sin(t * 7.3))
    firebox.keyframe_insert("energy", frame=fr)


def loco_world(t, local):
    loc, q = car_matrix(t, 4.3, -1.8, 6.95)
    return loc + q @ Vector(local), q


# ====================================================================== smoke
print("smoke ...")
puff_mesh = bpy.data.meshes.new("puff")
bm = bmesh.new()
bmesh.ops.create_icosphere(bm, subdivisions=2, radius=1.0)
bm.to_mesh(puff_mesh)
bm.free()
puff_mesh.materials.append(M["smoke"])
emit_t = []
t_ = 0.0
while t_ < T_HIT + 0.3:
    emit_t.append(t_)
    t_ += 0.11 if t_ > T0 else 0.16
LIFE = 3.6
wind = Vector((1.5, -1.0, 0))
for i, te in enumerate(emit_t):
    ob = bpy.data.objects.new(f"puff{i}", puff_mesh)
    scene.collection.objects.link(ob)
    p0, q = loco_world(te, (4.95, 0, 5.1))
    vt = (loco_world(te + 0.02, (4.95, 0, 5.1))[0] - loco_world(te - 0.02, (4.95, 0, 5.1))[0]) / 0.04
    jitter = Vector((rng.uniform(-1, 1), rng.uniform(-1, 1), rng.uniform(-0.5, 0.5)))
    f0 = max(1, int(te * FPS) + 1)
    ob.scale = (0.001, 0.001, 0.001)
    ob.color = (1, 1, 1, 0)
    ob.location = p0
    ob.keyframe_insert("scale", frame=f0 - 1)
    ob.keyframe_insert("color", frame=f0 - 1)
    ob.keyframe_insert("location", frame=f0 - 1)
    steps = int(LIFE * FPS / 3)
    for k in range(steps + 1):
        tau = k * 3 / FPS + (f0 - 1 - te * FPS) / FPS
        tau = max(tau, 0.0)
        pos = p0 + vt * 0.28 * 0.22 * (1 - math.exp(-tau / 0.22)) + Vector((0, 0, 1)) * (7.5 * (1 - math.exp(-tau / 0.9)) + 1.5 * tau) + wind * tau + jitter * tau * 0.8
        rad = 0.5 + 3.6 * (1 - math.exp(-tau / 1.3))
        dens = math.exp(-tau / 1.5) * clamp(tau / 0.06 + 0.2)
        fk = f0 + k * 3
        if fk > NFR:
            break
        ob.location = pos
        ob.scale = (rad, rad, rad * 0.85)
        ob.color = (1, 1, 1, dens)
        ob.keyframe_insert("location", frame=fk)
        ob.keyframe_insert("scale", frame=fk)
        ob.keyframe_insert("color", frame=fk)
    fend = f0 + steps * 3 + 1
    if fend <= NFR:
        ob.color = (1, 1, 1, 0)
        ob.keyframe_insert("color", frame=fend)

# brake-steam / cylinder drain puffs at the drop
for i in range(6):
    te = T0 - 0.9 + i * 0.12
    for side in (-1, 1):
        ob = bpy.data.objects.new(f"drain{i}{side}", puff_mesh)
        scene.collection.objects.link(ob)
        f0 = int(te * FPS) + 1
        p0, q = loco_world(te, (4.3, side * 1.6, 0.9))
        ob.scale = (0.001,) * 3
        ob.color = (1, 1, 1, 0)
        ob.location = p0
        for kk in (f0 - 1,):
            ob.keyframe_insert("scale", frame=kk)
            ob.keyframe_insert("color", frame=kk)
            ob.keyframe_insert("location", frame=kk)
        for k in range(12):
            tau = k * 3 / FPS
            ob.location = p0 + (q @ Vector((0, side * 1, 0))) * 3 * (1 - math.exp(-tau / 0.4)) + Vector((0, 0, 1.2 * tau))
            r_ = 0.4 + 2.2 * (1 - math.exp(-tau / 0.6))
            ob.scale = (r_,) * 3
            ob.color = (1, 1, 1, 0.8 * math.exp(-tau / 0.5))
            ob.keyframe_insert("location", frame=f0 + k * 3)
            ob.keyframe_insert("scale", frame=f0 + k * 3)
            ob.keyframe_insert("color", frame=f0 + k * 3)

# ====================================================================== sparks
print("sparks ...")
spark_mesh = bpy.data.meshes.new("spark")
bm = bmesh.new()
bmesh.ops.create_cube(bm, size=1.0, matrix=Matrix.Diagonal((0.05, 0.05, 0.05, 1)))
bm.to_mesh(spark_mesh)
bm.free()
spark_mesh.materials.append(M["spark"])
events = []
for sc_, sa, sl, *_ in JUMPS:
    for ob, a, b, back, wheels in CARS:
        for piv in (a, b):
            tl = t_at_s(sl + back - piv)
            events.append((tl, back, piv, 16 if ob is loco else 9))
# grinding sparks while leaning hard on curves
tt = T0 + 1
while tt < T_LAUNCH:
    if abs(lean(S(tt) - 6.95, tt)) > 0.2:
        events.append((tt, 6.95, -1.8, 3))
    tt += 0.06
si = 0
for tl, back, piv, n in events:
    if tl * FPS > NFR - 2:
        continue
    t1 = tl
    sc = S(t1) - back + piv
    base = Vector((tab(sc, 0), tab(sc, 1), z_track(min(sc, S_END)) + 0.05))
    F = fwd_vec(sc)
    v_tr = speed(t1)
    for k in range(n):
        ob = bpy.data.objects.new(f"spark{si}", spark_mesh)
        scene.collection.objects.link(ob)
        si += 1
        side = rng.choice((-0.75, 0.75))
        p0 = base + left_vec(sc) * side
        vel = F * v_tr * rng.uniform(0.3, 0.85) + left_vec(sc) * rng.uniform(-6, 6) * (1 if side > 0 else -1) + Vector((0, 0, rng.uniform(2, 9)))
        life = rng.uniform(0.3, 0.7)
        f0 = int(t1 * FPS) + 1
        ob.scale = (0.001,) * 3
        ob.location = p0
        ob.keyframe_insert("scale", frame=f0 - 1)
        ob.keyframe_insert("location", frame=f0 - 1)
        nfr = int(life * FPS)
        for j in range(nfr + 1):
            tau = j / FPS
            ob.location = p0 + vel * tau + Vector((0, 0, -4.9 * tau * tau))
            sz = 1.0 - tau / life
            ob.scale = (sz, sz, sz)
            ob.keyframe_insert("location", frame=f0 + j)
            ob.keyframe_insert("scale", frame=f0 + j)
        ob.scale = (0.001,) * 3
        ob.keyframe_insert("scale", frame=f0 + nfr + 1)
# a flash of orange light at each landing
for sc_, sa, sl, *_ in JUMPS:
    tl = t_at_s(sl + 6.95 - 4.3)
    lt = bpy.data.lights.new(f"land{tl:.2f}", "POINT")
    lt.color = (1.0, 0.5, 0.15)
    lo = bpy.data.objects.new(lt.name, lt)
    scene.collection.objects.link(lo)
    lo.location = Vector((tab(sl, 0), tab(sl, 1), z_track(sl) + 0.8))
    f0 = int(tl * FPS) + 1
    for fk, e in ((f0 - 1, 0), (f0, 40000), (f0 + 4, 9000), (f0 + 14, 0)):
        lt.energy = e
        lt.keyframe_insert("energy", frame=fk)

# ====================================================================== world, moon, lightning
print("world ...")
world = bpy.data.worlds.new("storm")
scene.world = world
pass
nt = world.node_tree
N = nt.nodes
bg = N["Background"]
tc = N.new("ShaderNodeTexCoord")
nz = N.new("ShaderNodeTexNoise")
nz.inputs["Scale"].default_value = 5.0
nz.inputs["Detail"].default_value = 14
nz.inputs["Distortion"].default_value = 0.25
nt.links.new(tc.outputs["Generated"], nz.inputs["Vector"])
ramp = N.new("ShaderNodeValToRGB")
ramp.color_ramp.elements[0].position = 0.42
ramp.color_ramp.elements[0].color = (0.012, 0.015, 0.024, 1)
ramp.color_ramp.elements[1].position = 0.68
ramp.color_ramp.elements[1].color = (0.09, 0.1, 0.13, 1)
nt.links.new(nz.outputs["Factor"], ramp.inputs["Factor"])
nt.links.new(ramp.outputs["Color"], bg.inputs["Color"])
strength = N.new("ShaderNodeValue")
strength.outputs[0].default_value = 1.6
nt.links.new(strength.outputs[0], bg.inputs["Strength"])
vol = N.new("ShaderNodeVolumePrincipled")
vol.inputs["Density"].default_value = 0.0025
vol.inputs["Anisotropy"].default_value = 0.55
vol.inputs["Color"].default_value = (0.7, 0.75, 0.9, 1)
# world fog disabled: it blacks out the scene in Eevee

moon = bpy.data.lights.new("moon", "SUN")
moon.energy = 3.2
moon.color = (0.62, 0.72, 1.0)
moon.angle = math.radians(1.5)
mo = bpy.data.objects.new("moon", moon)
scene.collection.objects.link(mo)
mo.rotation_euler = (math.radians(55), 0, math.radians(-40))

bolt_sun = bpy.data.lights.new("lightning", "SUN")
bolt_sun.color = (0.78, 0.84, 1.0)
bolt_sun.angle = math.radians(8)
bo = bpy.data.objects.new("lightning", bolt_sun)
scene.collection.objects.link(bo)
bo.rotation_euler = (math.radians(30), 0, math.radians(120))
keys = {}
for lt in LIGHTNING:
    f0 = int(lt * FPS) + 1
    for df, e in ((-1, 0), (0, 25), (1, 6), (2, 18), (3, 3), (6, 0)):
        keys[f0 + df] = max(keys.get(f0 + df, 0), e)
for fk in sorted(keys):
    if 1 <= fk <= NFR:
        bolt_sun.energy = keys[fk]
        bolt_sun.keyframe_insert("energy", frame=fk)
        strength.outputs[0].default_value = 1.6 + keys[fk] * 3
        strength.outputs[0].keyframe_insert("default_value", frame=fk)

# ====================================================================== camera
print("camera ...")
cam_data = bpy.data.cameras.new("cam")
cam_data.clip_start = 0.1
cam_data.clip_end = 6000
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam
cam.rotation_mode = "QUATERNION"

L1, L2, L3 = LANDINGS
CUTS = [
    (0.0, "side"), (T0, "front"), (BARS[2], "high"), (L1 - 1.3, "jump_side"),
    (L1 + 0.7, "wheels"), (BARS[7], "drone"), (L2 - 1.0, "landing_front"),
    (L2 + 0.55, "canyon_side"), (L3 - 1.4, "jump_high"), (L3 + 0.75, "front_low"),
    (T_LAUNCH - 1.5, "broken_end"),
]


def shot_at(t):
    name, t0 = CUTS[0][1], 0.0
    for tc_, n in CUTS:
        if t >= tc_:
            name, t0 = n, tc_
    return name, t0


def path_pt(s, lat=0.0, up=0.0):
    return Vector((tab(s, 0), tab(s, 1), z_track(min(s, S_END)))) + left_vec(s) * lat + Vector((0, 0, up))


def shake(t, amp):
    return Vector((noise.noise(Vector((t * 3.1, 0, 0))), noise.noise(Vector((0, t * 3.3, 7))), noise.noise(Vector((5, 0, t * 2.9))))) * amp


def landing_kick(t):
    k = 0.0
    for tl in LANDINGS:
        if 0 <= t - tl < 1.0:
            k += math.exp(-(t - tl) * 5)
    return k


def camera_at(t):
    name, ts = shot_at(t)
    lp, q = loco_world(t, (0, 0, 2.2))
    F = q @ Vector((1, 0, 0))
    sL = S(t) - 6.95
    lens, roll, amp = 28, 0.0, 0.05
    if name == "side":
        cpos = path_pt(sL + 4 + (t - ts) * 0.8, -11.5, 2.2)
        look = lp + F * 3
        lens, amp = 35, 0.02
    elif name == "front":
        cpos = path_pt(S(t) + 15, -4.5, 2.4)
        look = lp + F * 2
        lens, amp = 24, 0.06
    elif name == "high":
        cpos = path_pt(sL - 24, 9, 11)
        look = path_pt(S(t) + 30, 0, 0)
        lens, amp = 24, 0.07
        roll = -lean(sL, t) * 0.5
    elif name == "jump_side":
        s_c = JUMPS[0][0]
        cpos = path_pt(s_c + 14, -19, 6.5)
        look = lp
        lens, amp = 22, 0.02
    elif name == "wheels":
        cpos = lp + q @ Vector((2.5, -3.0, -1.4))
        look = lp + q @ Vector((-9, -0.6, -0.6))
        lens, amp = 18, 0.05
    elif name == "drone":
        cpos = path_pt(S(t) + 26, -14, 24)
        look = lp + F * -6
        lens, amp = 24, 0.06
        roll = 0.08 * math.sin(t * 1.3)
    elif name == "landing_front":
        sl = JUMPS[1][2]
        cpos = path_pt(sl + 27, 3.6, 1.0)
        look = lp + Vector((0, 0, 0.5))
        lens, amp = 22, 0.02
    elif name == "canyon_side":
        cpos = path_pt(sL + 2, -15, 4.5)
        look = lp + F * 4
        lens, amp = 26, 0.07
        roll = -lean(sL, t) * 0.4
    elif name == "jump_high":
        s_c = JUMPS[2][0]
        cpos = path_pt(s_c + 6, 11, 9)
        look = lp
        lens, amp = 22, 0.02
    elif name == "front_low":
        cpos = path_pt(S(t) + 11, 3.2, 1.3)
        look = lp + Vector((0, 0, 0.5))
        lens, amp = 20, 0.08
        roll = lean(sL, t) * 0.5
    else:  # broken_end: wait at the broken trestle, watch it launch, then stare into the storm
        cpos = path_pt(S_END + 5, -9.5, 1.5)
        lt_ = min(t, T_HIT - 0.2)
        target, _ = loco_world(lt_, (0, 0, 2.2))
        if t > T_HIT - 0.2:
            far = path_pt(S_END + 260, 0, 60)
            target = target.lerp(far, smooth((t - (T_HIT - 0.2)) / 3.0))
        look = target
        lens, amp = 22, 0.03
    amp_t = amp * (1 + 6 * landing_kick(t))
    if T_HIT <= t < T_HIT + 0.8:
        amp_t += 0.35 * math.exp(-(t - T_HIT) * 5)
    cpos = cpos + shake(t, amp_t)
    roll += 0.05 * landing_kick(t) * math.sin(t * 30)
    return cpos, look, lens, roll


prev = None
for fr in range(1, NFR + 1):
    t = (fr - 1) / FPS
    cpos, look, lens, roll = camera_at(t)
    d = (look - cpos).normalized()
    q = d.to_track_quat("-Z", "Y")
    q = q @ Quaternion((0, 0, 1), roll)
    if prev is not None and prev.dot(q) < 0:
        q = -q
    prev = q
    cam.location = cpos
    cam.rotation_quaternion = q
    cam.keyframe_insert("location", frame=fr)
    cam.keyframe_insert("rotation_quaternion", frame=fr)
    cam_data.lens = lens
    cam_data.keyframe_insert("lens", frame=fr)

# lightning bolts in the sky, placed in front of the camera at each strike
for i, lt in enumerate(LIGHTNING):
    f0 = int(lt * FPS) + 1
    if f0 > NFR:
        continue
    cpos, look, _, _ = camera_at(lt)
    d = (look - cpos)
    d.z = 0
    d = d.normalized() if d.length > 0 else Vector((1, 0, 0))
    side = Vector((-d.y, d.x, 0))
    r = random.Random(50 + i)
    top = cpos + d * 1400 + side * r.uniform(-500, 500) + Vector((0, 0, 700))
    pts = [top]
    p = top.copy()
    while p.z > cpos.z + 80:
        p = p + side * r.uniform(-40, 40) + d * r.uniform(-20, 20) + Vector((0, 0, -r.uniform(40, 80)))
        pts.append(p.copy())
    bmb = bmesh.new()
    for a, b in zip(pts, pts[1:]):
        v = b - a
        rot = Vector((0, 0, 1)).rotation_difference(v.normalized()).to_matrix().to_4x4()
        bmesh.ops.create_cone(bmb, cap_ends=False, segments=6, radius1=2.5, radius2=2.5, depth=v.length, matrix=Matrix.Translation((a + b) / 2) @ rot)
    ob = new_obj(f"bolt{i}", bmb, M["bolt"])
    for fk, hide in ((f0 - 1, True), (f0, False), (f0 + 2, True), (f0 + 3, False), (f0 + 5, True)):
        ob.hide_render = hide
        ob.keyframe_insert("hide_render", frame=fk)

# ====================================================================== render settings
r = scene.render
r.engine = "BLENDER_EEVEE"
r.resolution_x, r.resolution_y = 1920, 1080
r.resolution_percentage = 100
r.use_motion_blur = True
r.motion_blur_shutter = 0.55
try:
    r.motion_blur_position = "START"
except Exception as e:
    print("mb position", e)
ee = scene.eevee
ee.taa_render_samples = 24
ee.use_shadows = True
ee.use_raytracing = True
ee.volumetric_tile_size = "8"
ee.volumetric_samples = 48
ee.volumetric_start = 0.5
ee.volumetric_end = 900
ee.volumetric_sample_distribution = 0.85
ee.use_volumetric_shadows = True
ee.motion_blur_steps = 1
for vt in ("AgX", "Filmic", "Standard"):
    try:
        scene.view_settings.view_transform = vt
        break
    except Exception:
        pass
for look in ("AgX - Punchy", "Punchy", "AgX - Medium High Contrast", "Medium High Contrast"):
    try:
        scene.view_settings.look = look
        break
    except Exception:
        pass
scene.view_settings.exposure = 1.0
print("view", scene.view_settings.view_transform, scene.view_settings.look)
r.image_settings.file_format = "PNG"
r.image_settings.color_mode = "RGB"
r.filepath = os.path.join(HERE, "frames", "f_")
bpy.ops.wm.save_as_mainfile(filepath=os.path.join(HERE, "runaway.blend"))
print("saved; frames", NFR, "S_END", round(S_END, 1), "jumps", [(round(j[1]), round(j[2])) for j in JUMPS])
