"""The fighter: a hooded-scarf duelist drawn from a posed skeleton.

Each animation frame is a list of joint targets (hips, hands, feet, lean).
Two-bone IK places the elbows and knees, the parts are rasterised as pixel
capsules into a "material" map at half resolution (160x128), shaded and
outlined, then scaled 2x with nearest-neighbour to the 320x256 cell.

Cell rules from the asset checklist (all in 1x pixels, so halve them for
the 160x128 grid used here):
  * feet centre at (160, 240) -> (80, 120)
  * standing hurtbox x 120-200, y 40-240 -> x 60-100, y 20-120
  * crouch: head below y 120 -> below 60
  * light reach: fist to x 270 at y 80-110 -> x 135, y 40-55 (frame 02)
  * heavy reach: foot to x 300 at y 130-180 -> x 150, y 65-90 (frames 03-04)
"""

import math

import numpy as np
from PIL import Image, ImageDraw

W, H = 160, 128
SCALE = 2

# Material id -> draw depth. Higher z is drawn later (on top) and gets a dark
# contour line where it overlaps something lower.
MATERIALS = [
    # name, z
    ("scarf", 0),
    ("far_pants", 1),
    ("far_shoe", 1),
    ("far_sleeve", 1),
    ("far_skin", 1),
    ("pants", 2),
    ("top", 2),
    ("belt", 3),
    ("near_pants", 4),
    ("near_shoe", 4),
    ("collar", 5),
    ("skin", 6),
    ("hair", 7),
    ("band", 7),
    ("eye", 8),
    ("near_sleeve", 9),
    ("near_skin", 9),
]
MAT = {name: i + 1 for i, (name, _) in enumerate(MATERIALS)}
Z = np.zeros(len(MATERIALS) + 1, dtype=np.int32)
for i, (_, z) in enumerate(MATERIALS):
    Z[i + 1] = z

OUTLINE = (14, 12, 20)


def hex_rgb(h):
    h = h.lstrip("#")
    return tuple(int(h[i : i + 2], 16) for i in (0, 2, 4))


def mix(a, b, t):
    return tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(3))


# Two colour schemes. P1 is bright (white gi, cyan scarf), P2 is dark
# (violet gi, amber scarf, pale hair): they differ in brightness as well as
# hue, and neither is red, so the game's red hit flash stays visible.
SCHEMES = {
    "p1": {
        "top": "#e4ecf0",
        "pants": "#3a4868",
        "belt": "#1d2431",
        "scarf": "#38d0e6",
        "shoe": "#272c38",
        "skin": "#e0a77f",
        "hair": "#2a2132",
        "eye": "#120e18",
    },
    "p2": {
        "top": "#4b3468",
        "pants": "#201d2b",
        "belt": "#e3a536",
        "scarf": "#f0a43a",
        "shoe": "#131118",
        "skin": "#c4876a",
        "hair": "#e6e1d6",
        "eye": "#120e18",
    },
}


def palette(scheme):
    """Per material: (base, highlight, shadow, contour) colours."""
    s = {k: hex_rgb(v) for k, v in SCHEMES[scheme].items()}
    base = {
        "scarf": s["scarf"],
        "band": s["scarf"],
        "collar": s["scarf"],
        "top": s["top"],
        "near_sleeve": s["top"],
        "far_sleeve": mix(s["top"], (20, 18, 40), 0.35),
        "pants": s["pants"],
        "near_pants": s["pants"],
        "far_pants": mix(s["pants"], (8, 8, 16), 0.35),
        "belt": s["belt"],
        "near_shoe": s["shoe"],
        "far_shoe": mix(s["shoe"], (0, 0, 0), 0.3),
        "skin": s["skin"],
        "near_skin": s["skin"],
        "far_skin": mix(s["skin"], (40, 20, 40), 0.35),
        "hair": s["hair"],
        "eye": s["eye"],
    }
    pal = {}
    for name, c in base.items():
        hi = mix(c, (255, 250, 240), 0.35)
        sh = mix(c, (24, 16, 48), 0.35)
        ct = mix(c, OUTLINE, 0.7)
        pal[MAT[name]] = (c, hi, sh, ct)
    return pal


# ---------------------------------------------------------------- geometry


def v_add(a, b):
    return (a[0] + b[0], a[1] + b[1])


def v_scale(a, k):
    return (a[0] * k, a[1] * k)


def v_len(a):
    return math.hypot(a[0], a[1])


def two_bone(root, target, a, b, pref):
    """Joint position for a limb of lengths a, b from root toward target.
    Of the two possible bends, picks the one whose joint leans toward the
    `pref` direction. Unreachable targets give a straight limb."""
    dx, dy = target[0] - root[0], target[1] - root[1]
    d = v_len((dx, dy))
    d = max(min(d, a + b - 0.01), abs(a - b) + 0.01)
    base = math.atan2(dy, dx)
    cos_alpha = (a * a + d * d - b * b) / (2 * a * d)
    alpha = math.acos(max(-1.0, min(1.0, cos_alpha)))
    best = None
    for s in (1, -1):
        j = (root[0] + a * math.cos(base + s * alpha), root[1] + a * math.sin(base + s * alpha))
        score = (j[0] - root[0]) * pref[0] + (j[1] - root[1]) * pref[1]
        if best is None or score > best[0]:
            best = (score, j)
    joint = best[1]
    # End point actually reached (limb straightened toward an unreachable target).
    jd = (target[0] - joint[0], target[1] - joint[1])
    jl = v_len(jd) or 1.0
    end = (joint[0] + jd[0] / jl * b, joint[1] + jd[1] / jl * b)
    return joint, end


# ---------------------------------------------------------------- drawing


class Canvas:
    def __init__(self):
        self.img = Image.new("L", (W, H), 0)
        self.d = ImageDraw.Draw(self.img)

    def capsule(self, p, q, width, mat):
        m = MAT[mat]
        r = width / 2.0
        self.d.line([p, q], fill=m, width=max(1, int(round(width))))
        for c in (p, q):
            self.d.ellipse([c[0] - r + 0.5, c[1] - r + 0.5, c[0] + r - 0.5, c[1] + r - 0.5], fill=m)

    def circle(self, c, r, mat):
        self.d.ellipse([c[0] - r, c[1] - r, c[0] + r, c[1] + r], fill=MAT[mat])

    def pie(self, c, r, start, end, mat):
        self.d.pieslice([c[0] - r, c[1] - r, c[0] + r, c[1] + r], start, end, fill=MAT[mat])

    def poly(self, pts, mat):
        self.d.polygon(pts, fill=MAT[mat])


def shade(ids, pal):
    """Material map -> RGBA with top highlights, bottom shadows, contour
    lines where a part overlaps a lower one, and a 1-pixel outline."""
    h, w = ids.shape
    out = np.zeros((h, w, 4), dtype=np.uint8)
    pad = np.pad(ids, 1)
    up = pad[0:-2, 1:-1]
    down = pad[2:, 1:-1]
    left = pad[1:-1, 0:-2]
    right = pad[1:-1, 2:]
    for m, (base, hi, sh, ct) in pal.items():
        mask = ids == m
        if not mask.any():
            continue
        zm = Z[m]
        contour = np.zeros_like(mask)
        for n in (up, down, left, right):
            contour |= (n > 0) & (n != m) & (Z[n] < zm)
        top = (up != m) & ~contour
        bottom = ((down != m) | (right != m)) & ~contour & ~top
        layer = np.empty((h, w, 3), dtype=np.uint8)
        layer[:] = base
        layer[top] = hi
        layer[bottom] = sh
        layer[contour] = ct
        out[mask, :3] = layer[mask]
        out[mask, 3] = 255
    opaque = ids > 0
    p = np.pad(opaque, 1)
    near = np.zeros_like(opaque)
    for dy in (0, 1, 2):
        for dx in (0, 1, 2):
            near |= p[dy : dy + h, dx : dx + w]
    edge = near & ~opaque
    out[edge, :3] = OUTLINE
    out[edge, 3] = 255
    return out


# ---------------------------------------------------------------- the body

THIGH, SHIN = 23, 24
UPPER_ARM, FOREARM = 15, 14
TORSO = 30


def default_pose():
    return {
        "hip": (80, 73),
        "lean": 6,  # degrees, + leans forward (toward +x)
        "tilt": 0,  # head tilt, + nods forward
        "hand_n": (100, 50),
        "hand_f": (92, 44),
        "foot_n": (93, 116),
        "foot_f": (67, 116),
        "toe_n": 0,  # foot angle in degrees, 0 = toes forward, + = toes down
        "toe_f": 0,
        "knee_n": (1, -0.2),
        "knee_f": (1, -0.2),
        "elbow_n": (-0.4, 1),
        "elbow_f": (-0.4, 1),
        "scarf": 0.0,  # wave phase
        "scarf_lift": 0.0,  # + makes the scarf trail upward (falling, jumping)
        "fist_n": True,
        "fist_f": True,
    }


def pose(**kw):
    p = default_pose()
    p.update(kw)
    return p


def draw_pose(p, scheme):
    cv = Canvas()
    hip = p["hip"]
    lean = math.radians(p["lean"])
    up = (math.sin(lean), -math.cos(lean))  # along the spine, hip -> neck
    side = (-up[1], up[0])  # perpendicular, pointing "forward-ish"
    neck = v_add(hip, v_scale(up, TORSO))
    head = v_add(neck, v_scale(up, 9))
    shoulder = v_add(neck, v_scale(up, -3))
    sh_n = v_add(shoulder, v_scale(side, 1.5))
    sh_f = v_add(shoulder, v_scale(side, -2.5))
    hip_n = v_add(hip, (2, 0))
    hip_f = v_add(hip, (-2, 0))

    # Scarf: trails behind the neck and waves with the phase.
    pts = [v_add(neck, v_scale(up, -1))]
    for i in range(1, 7):
        wave = math.sin(p["scarf"] + i * 0.9) * (0.6 + i * 0.35)
        dx = -5.0 * i * math.cos(lean * 0.5)
        dy = 1.5 * i - p["scarf_lift"] * i + wave
        pts.append(v_add(pts[0], (dx, dy)))
    for i in range(len(pts) - 1):
        cv.capsule(pts[i], pts[i + 1], max(2, 6 - i * 0.7), "scarf")

    def leg(h, target, toe, pref, pants, shoe):
        knee, ankle = two_bone(h, target, THIGH, SHIN, pref)
        cv.capsule(h, knee, 10, pants)
        cv.capsule(knee, ankle, 8, pants)
        a = math.radians(toe)
        toe_pt = (ankle[0] + 7 * math.cos(a), ankle[1] + 1 + 7 * math.sin(a))
        cv.capsule((ankle[0] - 1, ankle[1] + 1), toe_pt, 5, shoe)

    def arm(s, target, pref, sleeve, skin, fist):
        elbow, wrist = two_bone(s, target, UPPER_ARM, FOREARM, pref)
        cv.capsule(s, elbow, 8, sleeve)
        cv.capsule(elbow, wrist, 6, sleeve)
        cv.circle(wrist, 3.4 if fist else 2.6, skin)

    # Far side first.
    arm(sh_f, p["hand_f"], p["elbow_f"], "far_sleeve", "far_skin", p["fist_f"])
    leg(hip_f, p["foot_f"], p["toe_f"], p["knee_f"], "far_pants", "far_shoe")

    # Torso: pelvis, a tapered chest, belt.
    cv.circle(hip, 7.5, "pants")
    waist = v_add(hip, v_scale(up, 7))
    chest = v_add(hip, v_scale(up, TORSO - 6))
    cv.capsule(waist, chest, 15, "top")
    cv.capsule(v_add(chest, v_scale(side, 1.5)), v_add(waist, v_scale(side, 1)), 12, "top")
    # Gi skirt flaps below the belt.
    flap = [
        v_add(waist, v_scale(side, 7)),
        v_add(waist, v_scale(side, -7)),
        v_add(v_add(hip, v_scale(up, -6)), v_scale(side, -8)),
        v_add(v_add(hip, v_scale(up, -4)), v_scale(side, 8)),
    ]
    cv.poly(flap, "top")
    belt_c = v_add(hip, v_scale(up, 5))
    cv.capsule(v_add(belt_c, v_scale(side, -7.5)), v_add(belt_c, v_scale(side, 7.5)), 3, "belt")

    leg(hip_n, p["foot_n"], p["toe_n"], p["knee_n"], "near_pants", "near_shoe")

    # Neck wrap of the scarf.
    cv.capsule(v_add(neck, v_scale(side, -3)), v_add(neck, v_scale(side, 3)), 6, "collar")

    # Head: face, hair over the top and back, headband, eye.
    tilt = math.radians(p["tilt"])
    face_dir = lean + tilt  # rotation of the head from upright
    deg = math.degrees(face_dir)
    cv.circle(head, 7, "skin")
    cv.pie(head, 7.6, 180 + deg, 340 + deg, "hair")
    cv.pie(head, 7.6, 95 + deg, 200 + deg, "hair")
    # Fringe tuft pointing forward.
    fr = (math.cos(face_dir - 0.5), math.sin(face_dir - 0.5))
    cv.capsule(v_add(head, v_scale(fr, 3)), v_add(head, v_scale(fr, 8)), 3, "hair")
    hb = (math.cos(face_dir - 0.35), math.sin(face_dir - 0.35))
    hb_back = v_add(head, v_scale(hb, -7))
    cv.capsule(hb_back, v_add(head, v_scale(hb, 6.5)), 2, "band")
    # Headband tails flutter behind.
    tail1 = v_add(hb_back, (-5, 1 + math.sin(p["scarf"] + 1) * 1.5 - p["scarf_lift"] * 2))
    tail2 = v_add(hb_back, (-6, 4 + math.sin(p["scarf"] + 2) * 1.5 - p["scarf_lift"] * 2))
    cv.capsule(hb_back, tail1, 2, "band")
    cv.capsule(hb_back, tail2, 2, "band")
    eye = v_add(head, (math.cos(face_dir) * 3.6 - math.sin(face_dir) * 0.6, math.sin(face_dir) * 3.6 + math.cos(face_dir) * 0.6))
    ex, ey = int(round(eye[0])), int(round(eye[1]))
    cv.d.rectangle([ex, ey - 1, ex, ey], fill=MAT["eye"])

    arm(sh_n, p["hand_n"], p["elbow_n"], "near_sleeve", "near_skin", p["fist_n"])

    ids = np.array(cv.img, dtype=np.int32)
    rgba = shade(ids, palette(scheme))
    img = Image.fromarray(rgba, "RGBA")
    return img.resize((W * SCALE, H * SCALE), Image.NEAREST)


# ---------------------------------------------------------------- animations

STANCE_N = (93, 116)
STANCE_F = (67, 116)


def lerp(a, b, t):
    if isinstance(a, tuple):
        return tuple(lerp(x, y, t) for x, y in zip(a, b))
    if isinstance(a, bool):
        return a if t < 0.5 else b
    return a + (b - a) * t


def blend(p, q, t):
    return {k: lerp(p[k], q[k], t) for k in p}


def idle():
    frames = []
    for i in range(8):
        t = i / 8 * 2 * math.pi
        bob = math.sin(t) * 1.2
        frames.append(
            pose(
                hip=(80, 73 + bob),
                lean=6 + math.sin(t) * 0.8,
                hand_n=(100, 50 + bob * 0.8),
                hand_f=(92, 44 + bob * 0.6),
                scarf=t,
            )
        )
    return frames


def walk():
    frames = []
    for i in range(8):
        t = i / 8

        def foot(phase):
            phase %= 1.0
            if phase < 0.5:  # planted: slides back under the body as it moves
                return (96 - 64 * phase, 116), 0
            s = (phase - 0.5) * 2
            return (64 + 32 * s, 116 - 6 * math.sin(math.pi * s)), 10 * math.sin(math.pi * s)

        fn, tn = foot(t)
        ff, tf = foot(t + 0.5)
        hip_y = 74 - 2 * abs(math.sin(2 * math.pi * t))
        swing = math.sin(2 * math.pi * t)
        frames.append(
            pose(
                hip=(80, hip_y),
                lean=8,
                foot_n=fn,
                foot_f=ff,
                toe_n=tn,
                toe_f=tf,
                hand_n=(101 + swing * 2, 50 + (hip_y - 73)),
                hand_f=(93 - swing * 2, 45 + (hip_y - 73)),
                scarf=t * 2 * math.pi,
            )
        )
    return frames


def crouch():
    down = pose(
        hip=(73, 98),
        lean=45,
        tilt=-20,
        hand_n=(98, 74),
        hand_f=(92, 68),
        foot_n=(92, 116),
        foot_f=(62, 116),
        scarf=1.0,
    )
    up = default_pose()
    return [blend(up, down, 0.4), blend(up, down, 0.75), down]


def jump():
    return [
        pose(hip=(80, 70), lean=4, hand_n=(98, 40), hand_f=(88, 36), foot_n=(88, 118), foot_f=(73, 118), toe_n=55, toe_f=55, scarf_lift=0.3),
        pose(hip=(80, 74), lean=10, hand_n=(100, 46), hand_f=(90, 42), foot_n=(94, 102), foot_f=(70, 108), toe_n=35, toe_f=30, scarf_lift=0.5, scarf=1),
        pose(hip=(80, 80), lean=22, tilt=-10, hand_n=(100, 60), hand_f=(94, 54), foot_n=(90, 100), foot_f=(74, 102), toe_n=30, toe_f=30, scarf_lift=0.2, scarf=2),
        pose(hip=(80, 76), lean=10, hand_n=(102, 48), hand_f=(84, 46), foot_n=(94, 108), foot_f=(70, 112), toe_n=30, toe_f=35, scarf_lift=-0.6, scarf=3),
        pose(hip=(80, 72), lean=4, hand_n=(104, 46), hand_f=(70, 50), foot_n=(92, 117), foot_f=(68, 115), toe_n=20, toe_f=25, scarf_lift=-0.8, scarf=4),
    ]


def land():
    return [
        pose(hip=(78, 86), lean=22, hand_n=(98, 64), hand_f=(90, 58), scarf_lift=-0.5),
        pose(hip=(79, 79), lean=12, hand_n=(99, 56), hand_f=(91, 50), scarf_lift=-0.2),
    ]


def light():
    return [
        pose(hip=(78, 74), lean=2, hand_n=(92, 45), hand_f=(88, 47), scarf=0.5),
        pose(hip=(84, 75), lean=15, hand_n=(114, 46), hand_f=(94, 48), foot_n=(99, 116), scarf=1.0),
        # Frame 02 is the active frame: fist's far edge at x 135, y ~47.
        pose(hip=(91, 76), lean=26, tilt=-14, hand_n=(140, 47), hand_f=(96, 54), foot_n=(105, 116), foot_f=(64, 116), elbow_n=(0, 1), scarf=1.5),
        pose(hip=(87, 75), lean=18, tilt=-6, hand_n=(117, 46), hand_f=(94, 50), foot_n=(102, 116), scarf=2.0),
        pose(hip=(83, 74), lean=11, hand_n=(104, 47), hand_f=(93, 46), foot_n=(97, 116), scarf=2.5),
        pose(hip=(80, 73), lean=7, hand_n=(100, 49), hand_f=(92, 45), scarf=3.0),
    ]


def heavy():
    kick = dict(knee_n=(0.2, -1))
    return [
        pose(hip=(76, 74), lean=-4, hand_n=(96, 48), hand_f=(88, 44), foot_n=(92, 116), foot_f=(66, 116), scarf=0.3),
        pose(hip=(74, 72), lean=-10, hand_n=(94, 46), hand_f=(86, 46), foot_n=(90, 98), toe_n=-15, foot_f=(66, 116), knee_n=(1, -1), scarf=0.6),
        pose(hip=(74, 71), lean=-16, hand_n=(90, 44), hand_f=(84, 48), foot_n=(84, 90), toe_n=10, foot_f=(68, 116), knee_n=(1, -1), scarf=0.9),
        # Frames 03-04 are active: toe tip at x ~150, y ~77.
        pose(hip=(94, 74), lean=-26, tilt=8, hand_n=(98, 50), hand_f=(76, 58), foot_n=(141, 77), toe_n=0, foot_f=(74, 116), scarf=1.2, scarf_lift=0.2, **kick),
        pose(hip=(94, 74), lean=-28, tilt=8, hand_n=(97, 51), hand_f=(75, 60), foot_n=(142, 78), toe_n=2, foot_f=(74, 116), scarf=1.6, scarf_lift=0.2, **kick),
        pose(hip=(88, 73), lean=-18, hand_n=(96, 48), hand_f=(80, 52), foot_n=(100, 92), toe_n=10, foot_f=(72, 116), knee_n=(1, -1), scarf=2.0),
        pose(hip=(84, 73), lean=-10, hand_n=(97, 48), hand_f=(84, 48), foot_n=(98, 104), toe_n=10, foot_f=(70, 116), knee_n=(1, -0.5), scarf=2.4),
        pose(hip=(81, 74), lean=-2, hand_n=(98, 49), hand_f=(88, 46), foot_n=(95, 116), foot_f=(68, 116), scarf=2.8),
        pose(hip=(80, 73), lean=4, hand_n=(99, 50), hand_f=(91, 45), scarf=3.2),
    ]


def hit_high():
    return [
        pose(hip=(74, 74), lean=-22, tilt=-25, hand_n=(90, 40), hand_f=(70, 46), foot_n=(90, 116), foot_f=(62, 116), fist_n=False, fist_f=False, scarf=0.5, scarf_lift=0.4),
        pose(hip=(72, 75), lean=-26, tilt=-30, hand_n=(88, 38), hand_f=(68, 44), foot_n=(90, 116), foot_f=(62, 116), fist_n=False, fist_f=False, scarf=1.0, scarf_lift=0.5),
        pose(hip=(75, 74), lean=-12, tilt=-12, hand_n=(94, 46), hand_f=(80, 46), foot_n=(91, 116), foot_f=(64, 116), scarf=1.5),
        pose(hip=(78, 73), lean=0, tilt=-4, hand_n=(98, 49), hand_f=(88, 45), scarf=2.0),
    ]


def hit_mid():
    return [
        pose(hip=(72, 78), lean=48, tilt=18, hand_n=(90, 82), hand_f=(84, 84), foot_n=(88, 116), foot_f=(60, 116), elbow_n=(0, 1), scarf=0.5, scarf_lift=0.5),
        pose(hip=(70, 79), lean=54, tilt=20, hand_n=(88, 84), hand_f=(82, 86), foot_n=(88, 116), foot_f=(60, 116), elbow_n=(0, 1), scarf=1.0, scarf_lift=0.6),
        pose(hip=(74, 77), lean=30, tilt=10, hand_n=(94, 66), hand_f=(86, 64), foot_n=(90, 116), foot_f=(62, 116), scarf=1.5),
        pose(hip=(78, 74), lean=14, hand_n=(98, 54), hand_f=(90, 50), scarf=2.0),
    ]


def hit_air():
    return [
        pose(hip=(78, 76), lean=-35, tilt=-20, hand_n=(92, 34), hand_f=(66, 40), foot_n=(100, 106), foot_f=(88, 114), toe_n=30, toe_f=40, fist_n=False, fist_f=False, scarf_lift=0.8, scarf=0.4),
        pose(hip=(78, 77), lean=-40, tilt=-24, hand_n=(88, 32), hand_f=(62, 44), foot_n=(102, 104), foot_f=(90, 112), toe_n=30, toe_f=40, fist_n=False, fist_f=False, scarf_lift=0.9, scarf=1.4),
    ]


LYING = dict(hip=(68, 112), lean=-90, tilt=0, hand_n=(38, 116), hand_f=(46, 110), foot_n=(112, 116), foot_f=(106, 114), toe_n=-60, toe_f=-60, knee_n=(0, -1), knee_f=(0, -1), elbow_n=(0, -1), elbow_f=(0, -1), fist_n=False, fist_f=False, scarf=0.0)


def knockdown():
    lying = pose(**LYING)
    bounce = pose(**{**LYING, "hip": (68, 108), "lean": -84, "foot_n": (112, 106), "foot_f": (106, 102), "scarf_lift": 0.5})
    return [
        pose(hip=(72, 76), lean=-30, tilt=-25, hand_n=(88, 38), hand_f=(66, 44), foot_n=(90, 116), foot_f=(62, 116), fist_n=False, fist_f=False, scarf_lift=0.5),
        pose(hip=(66, 84), lean=-50, tilt=-20, hand_n=(60, 52), hand_f=(54, 60), foot_n=(98, 108), foot_f=(88, 114), toe_n=-20, toe_f=10, fist_n=False, fist_f=False, scarf_lift=0.9, scarf=1),
        pose(hip=(60, 96), lean=-72, tilt=-10, hand_n=(44, 78), hand_f=(50, 72), foot_n=(102, 96), foot_f=(96, 104), toe_n=-40, toe_f=-30, knee_n=(0, -1), knee_f=(0, -1), fist_n=False, fist_f=False, scarf_lift=1.0, scarf=2),
        lying,
        bounce,
        lying,
    ]


def down():
    a = pose(**LYING)
    b = pose(**{**LYING, "lean": -88, "hip": (68, 111.5), "scarf": 0.6})
    return [a, b]


def intro():
    upright = pose(hip=(80, 72), lean=0, hand_n=(84, 96), hand_f=(77, 96), foot_n=(86, 116), foot_f=(74, 116), fist_n=False, fist_f=False, elbow_n=(0, 1), elbow_f=(0, 1))
    bow = pose(hip=(78, 73), lean=28, tilt=10, hand_n=(98, 70), hand_f=(96, 70), foot_n=(86, 116), foot_f=(74, 116), elbow_n=(0, 1), elbow_f=(0, 1), scarf=1)
    bow2 = pose(**{**bow, "lean": 36, "tilt": 14, "hand_n": (100, 76), "hand_f": (98, 76), "scarf": 1.6})
    rise = pose(hip=(79, 72), lean=10, hand_n=(98, 54), hand_f=(92, 50), foot_n=(88, 116), foot_f=(70, 116), scarf=2.2)
    step = pose(hip=(80, 74), lean=8, hand_n=(104, 48), hand_f=(92, 44), scarf=2.8)
    return [upright, bow, bow2, rise, step, default_pose()]


def win():
    hip_hand = (72, 72)
    return [
        pose(scarf=0.5),
        pose(hip=(80, 72), lean=2, hand_n=(104, 34), hand_f=hip_hand, elbow_f=(-1, 0), scarf=1.2),
        pose(hip=(80, 71), lean=-2, tilt=-10, hand_n=(94, 10), hand_f=hip_hand, elbow_n=(1, 0), elbow_f=(-1, 0), foot_n=(90, 116), foot_f=(70, 116), scarf=2.0, scarf_lift=0.4),
        pose(hip=(80, 71), lean=-3, tilt=-12, hand_n=(93, 8), hand_f=hip_hand, elbow_n=(1, 0), elbow_f=(-1, 0), foot_n=(90, 116), foot_f=(70, 116), scarf=2.8, scarf_lift=0.5),
        pose(hip=(80, 71), lean=-3, tilt=-12, hand_n=(93, 8), hand_f=hip_hand, elbow_n=(1, 0), elbow_f=(-1, 0), foot_n=(90, 116), foot_f=(70, 116), scarf=3.6, scarf_lift=0.4),
        pose(hip=(80, 71), lean=-3, tilt=-12, hand_n=(93, 8), hand_f=hip_hand, elbow_n=(1, 0), elbow_f=(-1, 0), foot_n=(90, 116), foot_f=(70, 116), scarf=4.4, scarf_lift=0.3),
    ]


def turn():
    # Mid-turn: body square to the camera-ish, arms in close.
    return [
        pose(hip=(80, 73), lean=2, hand_n=(90, 50), hand_f=(86, 48), foot_n=(86, 116), foot_f=(72, 116), scarf=1),
        pose(hip=(80, 73), lean=4, hand_n=(96, 50), hand_f=(90, 46), foot_n=(90, 116), foot_f=(70, 116), scarf=2),
    ]


ANIMATIONS = {
    "idle": idle,
    "walk": walk,
    "crouch": crouch,
    "jump": jump,
    "land": land,
    "light": light,
    "heavy": heavy,
    "hit_high": hit_high,
    "hit_mid": hit_mid,
    "hit_air": hit_air,
    "knockdown": knockdown,
    "down": down,
    "intro": intro,
    "win": win,
    "turn": turn,
}
