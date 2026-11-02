"""Generates every PNG and the font in game/assets/ from code.

Run from the repo root:
    python scripts/art/gen_art.py

Needs Python 3 with Pillow, numpy and fontTools (SETUP.md, "Art"). The
output is deterministic (fixed random seeds), so re-running it only changes
files whose drawing code changed. Any file can instead be replaced by hand
(for example art made in another tool): keep the same path and size and the
game picks it up on the next build.

Sizes, anchors and frame counts follow the "GhostFrame art asset checklist".
Everything is pixel art drawn on a half-resolution grid and scaled 2x with
nearest-neighbour, so one art pixel is 2x2 screen pixels at 1280x720.
"""

import math
import os
import random
import sys

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fighter  # noqa: E402
import pixfont  # noqa: E402

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, "game", "assets")

# Palette slots from the checklist.
P1_COLOR = (56, 208, 230)  # cyan, bright
P2_COLOR = (240, 164, 58)  # amber, darker scheme overall
ACCENT = (158, 240, 106)  # UI accent: cursor, active slot, FIGHT
HEALTH = (244, 206, 78)
DAMAGE = (232, 86, 64)
TEXT = (232, 236, 244)
INK = (14, 12, 20)


def out_path(rel):
    path = os.path.join(OUT, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    return path


def save(img, rel):
    img.save(out_path(rel), optimize=True)


def up2(img):
    return img.resize((img.width * 2, img.height * 2), Image.NEAREST)


def mix(a, b, t):
    return tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(len(a)))


BAYER4 = np.array([[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]]) / 16.0


def dither_gradient(w, h, stops, steps=8):
    """Vertical gradient through `stops` [(t, rgb)], quantised to `steps`
    bands with ordered dithering between them, for a pixel-art sky."""
    ys = np.linspace(0, 1, h)
    out = np.zeros((h, w, 3), dtype=np.uint8)
    for y in range(h):
        t = ys[y]
        for i in range(len(stops) - 1):
            if stops[i][0] <= t <= stops[i + 1][0]:
                a, b = stops[i], stops[i + 1]
                local = (t - a[0]) / max(1e-6, b[0] - a[0])
                break
        q = local * steps
        lo = math.floor(q)
        frac = q - lo
        for x in range(w):
            step = lo + (1 if frac > BAYER4[y % 4, x % 4] else 0)
            out[y, x] = mix(a[1], b[1], min(step, steps) / steps)
    return out


def text_mask(text, scale=1):
    w = pixfont.text_width(text)
    h = pixfont.CAP_ROWS + pixfont.DESC_ROWS
    m = np.zeros((h, w), dtype=bool)
    for x, y in pixfont.text_pixels(text):
        m[y, x] = True
    if not any(len(pixfont.glyph(c)) > pixfont.CAP_ROWS for c in text):
        m = m[: pixfont.CAP_ROWS]
    return np.kron(m, np.ones((scale, scale), dtype=bool))


def dilate(mask, r):
    h, w = mask.shape
    p = np.pad(mask, r)
    out = np.zeros_like(mask)
    for dy in range(-r, r + 1):
        for dx in range(-r, r + 1):
            if dx * dx + dy * dy <= r * r + r:
                out |= p[r + dy : r + dy + h, r + dx : r + dx + w]
    return out


def styled_text(text, scale, top, bottom, outline=INK, border=None, shadow=None, gloss=True):
    """Pixel text: vertical gradient fill, dark outline, optional drop shadow."""
    border = border or max(2, scale // 3)
    shadow = scale // 2 if shadow is None else shadow
    m = text_mask(text, scale)
    pad = border + shadow + 2
    m = np.pad(m, pad)
    h, w = m.shape
    img = np.zeros((h, w, 4), dtype=np.uint8)
    ol = dilate(m, border)
    if shadow:
        sh = np.zeros_like(ol)
        sh[shadow:, shadow:] = ol[:-shadow, :-shadow]
        img[sh] = (0, 0, 0, 150)
    img[ol] = (*outline, 255)
    rows = np.where(m.any(1))[0]
    y0, y1 = rows.min(), rows.max()
    for y in range(y0, y1 + 1):
        t = (y - y0) / max(1, y1 - y0)
        # Banded gradient, 6 flat steps: reads as pixel art, not airbrush.
        c = mix(top, bottom, math.floor(t * 6) / 5 if t < 1 else 1)
        if gloss and (y - y0) < max(1, scale // 4):
            c = mix(c, (255, 255, 255), 0.5)
        row = m[y]
        img[y, row] = (*c, 255)
    return Image.fromarray(img, "RGBA")


def fit(img, max_w, max_h):
    """Pads (never scales) an image to its cell, centred."""
    assert img.width <= max_w and img.height <= max_h, (img.size, max_w, max_h)
    return img


# ---------------------------------------------------------------- fighter


def gen_fighter():
    for scheme in ("p1", "p2"):
        for name, fn in fighter.ANIMATIONS.items():
            for i, p in enumerate(fn()):
                save(fighter.draw_pose(p, scheme), f"fighter/{scheme}/{name}/{i:02}.png")


# ---------------------------------------------------------------- stage
# Low-res stage is 672x392; the visible screen is (16,16)-(656,376) and the
# floor surface is y = 336 (1x: 672 in the image, 640 on screen).

SW, SH = 672, 392
FLOOR = 336


def gen_stage():
    rng = random.Random(11)

    # Far: night sky, moon, stars, two ranges of mountains, mist.
    sky = dither_gradient(
        SW,
        SH,
        [(0.0, (16, 14, 34)), (0.45, (38, 30, 72)), (0.8, (78, 56, 104)), (1.0, (96, 70, 112))],
        steps=10,
    )
    img = Image.fromarray(sky, "RGB").convert("RGBA")
    d = ImageDraw.Draw(img)
    for _ in range(160):
        x, y = rng.randrange(SW), rng.randrange(80, 250)
        c = mix((60, 56, 96), (190, 190, 220), rng.random() ** 3)
        d.point((x, y), fill=c)
    # Moon with a halo, off to the right and below the HUD band.
    mx, my = 520, 110
    for r, c in ((34, (70, 58, 108)), (28, (88, 74, 124)), (22, (206, 204, 222))):
        d.ellipse([mx - r, my - r, mx + r, my + r], fill=c)
    for cx, cy, r in ((512, 104, 5), (528, 118, 3), (522, 98, 2)):
        d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=(180, 178, 204))

    def ridge(base, amp, freq, seed, color):
        r = random.Random(seed)
        phases = [r.random() * 6.28 for _ in range(4)]
        pts = [(0, SH)]
        for x in range(0, SW + 1, 2):
            y = base
            for k, ph in enumerate(phases):
                y -= amp / (k + 1) * (0.5 + 0.5 * math.sin(x * freq * (k + 1) + ph))
            pts.append((x, int(y)))
        pts.append((SW, SH))
        d.polygon(pts, fill=color)

    ridge(270, 70, 0.012, 3, (58, 44, 86))
    ridge(300, 46, 0.02, 5, (48, 36, 72))
    # Mist band across the base of the mountains.
    for y in range(282, 312):
        for x in range(SW):
            if (y - 282) / 30 > BAYER4[y % 4, x % 4] * 0.9 + 0.1 and (x // 3 + y) % 5:
                d.point((x, y), fill=(84, 66, 110))
    ridge(326, 18, 0.03, 9, (38, 30, 58))
    save(up2(img), "stage/bg_far.png")

    # Mid: temple silhouettes at the sides, a low wall across the middle so
    # the fighting area stays calm. Transparent everywhere else.
    mid = Image.new("RGBA", (SW, SH), (0, 0, 0, 0))
    d = ImageDraw.Draw(mid)
    sil = (28, 22, 44, 255)
    sil_hi = (44, 34, 64, 255)

    def pagoda(cx, base, tiers):
        w = 64
        y = base
        for t in range(tiers):
            h = 22
            d.rectangle([cx - w // 2 + 8, y - h, cx + w // 2 - 8, y], fill=sil)
            # Upswept roof.
            d.polygon([(cx - w // 2 - 6, y - h), (cx + w // 2 + 6, y - h), (cx + w // 2 - 4, y - h - 10), (cx - w // 2 + 4, y - h - 10)], fill=sil)
            d.line([(cx - w // 2 - 6, y - h), (cx + w // 2 + 6, y - h)], fill=sil_hi)
            for wx in range(cx - w // 2 + 14, cx + w // 2 - 12, 10):
                d.rectangle([wx, y - h + 7, wx + 3, y - h + 12], fill=(150, 112, 70, 255))
            y -= h + 10
            w -= 12
        d.line([(cx, y), (cx, y - 18)], fill=sil, width=2)

    pagoda(78, 330, 4)
    # Torii gate on the right.
    gx = 586
    d.rectangle([gx - 34, 236, gx - 28, 334], fill=sil)
    d.rectangle([gx + 28, 236, gx + 34, 334], fill=sil)
    d.polygon([(gx - 52, 228), (gx + 52, 228), (gx + 46, 236), (gx - 46, 236)], fill=sil)
    d.rectangle([gx - 42, 248, gx + 42, 252], fill=sil)
    d.line([(gx - 52, 228), (gx + 52, 228)], fill=sil_hi)
    # Pines.
    for px, top in ((150, 214), (176, 240), (500, 226), (650, 200), (14, 230)):
        d.rectangle([px - 2, top + 30, px + 1, 334], fill=sil)
        for k in range(4):
            yy = top + k * 16
            ww = 10 + k * 5
            d.polygon([(px - ww, yy + 14), (px + ww, yy + 14), (px, yy - 4)], fill=sil)
    # Low stone wall with paper lanterns between the buildings.
    d.rectangle([0, 318, SW, 336], fill=(46, 38, 62, 255))
    d.line([(0, 318), (SW, 318)], fill=(70, 60, 88, 255))
    for x in range(0, SW, 24):
        d.line([(x, 319), (x, 336)], fill=(38, 30, 52, 255))
    for lx in (128, 256, 416, 544):
        d.rectangle([lx - 1, 300, lx, 318], fill=(38, 30, 52, 255))
        d.ellipse([lx - 6, 288, lx + 5, 302], fill=(214, 150, 84, 255))
        d.ellipse([lx - 4, 290, lx + 3, 297], fill=(240, 196, 120, 255))
        d.rectangle([lx - 4, 286, lx + 3, 288], fill=(38, 30, 52, 255))
    save(up2(mid), "stage/bg_mid.png")

    # Floor: wooden courtyard boards with a clear lit top edge at y 336.
    fl = Image.new("RGBA", (SW, SH), (0, 0, 0, 0))
    d = ImageDraw.Draw(fl)
    y = FLOOR
    band = 0
    heights = [4, 5, 6, 7, 8, 9, 10, 12]
    shades = [(104, 82, 76), (92, 72, 68), (98, 77, 72), (86, 67, 64)]
    while y < SH:
        h = heights[min(band, len(heights) - 1)]
        c = shades[band % len(shades)]
        d.rectangle([0, y, SW, y + h - 1], fill=(*c, 255))
        d.line([(0, y + h - 1), (SW, y + h - 1)], fill=(60, 46, 48, 255))
        # Board seams, staggered per band; spacing grows toward the viewer.
        step = 40 + band * 8
        off = (band * 23) % step
        for x in range(off, SW, step):
            d.line([(x, y), (x, y + h - 2)], fill=(64, 50, 50, 255))
        y += h
        band += 1
    d.line([(0, FLOOR), (SW, FLOOR)], fill=(176, 150, 132, 255))
    d.line([(0, FLOOR + 1), (SW, FLOOR + 1)], fill=(130, 104, 94, 255))
    save(up2(fl), "stage/floor.png")

    # Foreground: grass tufts and two stone lantern caps in the bottom strip
    # (last 20 art pixels of the screen) and the outer corners only.
    fg = Image.new("RGBA", (SW, SH), (0, 0, 0, 0))
    d = ImageDraw.Draw(fg)
    r = random.Random(4)
    for x in list(range(0, 46)) + list(range(626, SW)):
        hgt = r.randrange(4, 14)
        d.line([(x, 392), (x + r.choice((-1, 0, 1)), 376 + 4 - hgt + 6)], fill=(26, 34, 30, 255))
    for x in range(0, SW, 3):
        hgt = r.randrange(1, 5)
        d.line([(x, 392), (x, 376 - hgt + 4)], fill=(30, 38, 34, 255))
    save(up2(fg), "stage/fg.png")


# ---------------------------------------------------------------- effects


def burst(size, frame, frames, spikes, inner, outer, bias):
    """One frame of a star-burst spark on a low-res `size` grid."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    c = size / 2
    t = frame / (frames - 1)
    grow = 0.45 + 0.55 * min(1.0, t * 1.8)
    fade = 1.0 - max(0.0, t - 0.4) / 0.6
    rr = random.Random(spikes * 31 + size)
    angles = [i * 2 * math.pi / spikes + rr.uniform(-0.2, 0.2) for i in range(spikes)]
    for k, a in enumerate(angles):
        fwd = 1.0 + bias * max(0.0, math.cos(a))
        length = (c - 2) * grow * fwd * (0.75 if k % 2 else 1.0)
        length = min(length, c - 1)
        width = max(1.0, (3.2 if k % 2 == 0 else 2.0) * fade * (size / 48))
        tip = (c + math.cos(a) * length, c + math.sin(a) * length)
        base_l = (c + math.cos(a + 0.5) * width, c + math.sin(a + 0.5) * width)
        base_r = (c + math.cos(a - 0.5) * width, c + math.sin(a - 0.5) * width)
        d.polygon([base_l, tip, base_r], fill=outer if k % 2 else inner)
    core = (size / 7) * (1.4 - t)
    if core > 0.8:
        d.ellipse([c - core - 1, c - core - 1, c + core + 1, c + core + 1], fill=inner)
        d.ellipse([c - core * 0.6, c - core * 0.6, c + core * 0.6, c + core * 0.6], fill=(255, 255, 255, 255))
    if t > 0.3:
        ring = (c - 2) * (0.35 + 0.65 * t)
        d.ellipse([c - ring, c - ring, c + ring, c + ring], outline=(*outer[:3], int(220 * fade)), width=1)
    return img


def gen_fx():
    for i in range(4):
        save(up2(burst(48, i, 4, 8, (255, 250, 210, 255), (255, 226, 120, 255), 0.6)), f"fx/spark_light/{i:02}.png")
    for i in range(6):
        save(up2(burst(80, i, 6, 12, (255, 244, 190, 255), (255, 150, 52, 255), 0.4)), f"fx/spark_heavy/{i:02}.png")

    sh = Image.new("RGBA", (48, 10), (0, 0, 0, 0))
    d = ImageDraw.Draw(sh)
    d.ellipse([0, 0, 47, 9], fill=(8, 6, 14, 70))
    d.ellipse([6, 1, 41, 8], fill=(8, 6, 14, 110))
    d.ellipse([14, 2, 33, 7], fill=(8, 6, 14, 140))
    save(up2(sh), "fx/shadow.png")

    for kind, spread in (("dust_jump", 1.0), ("dust_land", 1.5)):
        for i in range(3):
            img = Image.new("RGBA", (64, 24), (0, 0, 0, 0))
            d = ImageDraw.Draw(img)
            t = i / 2
            a = int(230 * (1 - t * 0.6))
            for side in (-1, 1):
                for k in range(3):
                    cx = 32 + side * (6 + k * 7 + t * 10 * spread)
                    r = 3 + k * 0.6 + t * 3
                    cy = 21 - k * 1.5 - t * 4
                    d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=(190, 176, 168, a))
                    d.ellipse([cx - r + 1, cy - r, cx + r - 2, cy], fill=(226, 216, 206, a))
            save(up2(img), f"fx/{kind}/{i:02}.png")


# ---------------------------------------------------------------- HUD


def gen_hud():
    # Bar frame, P1's (the game mirrors it for P2). 288x20 low-res.
    f = Image.new("RGBA", (288, 20), (0, 0, 0, 0))
    d = ImageDraw.Draw(f)
    d.polygon([(0, 0), (287, 0), (279, 19), (0, 19)], fill=(*INK, 255))
    d.polygon([(1, 1), (285, 1), (278, 18), (1, 18)], fill=(118, 112, 146, 255))
    d.line([(1, 1), (285, 1)], fill=(184, 180, 208, 255))
    d.line([(1, 18), (278, 18)], fill=(70, 64, 92, 255))
    # Dark backing where the fill goes (fill sits at +8,+8 at 1x = +4,+4 here).
    d.rectangle([3, 3, 284, 16], fill=(*INK, 255))
    d.rectangle([4, 4, 283, 15], fill=(36, 30, 48, 255))
    # Rivets.
    for x in (2, 284):
        d.point((x, 9), fill=(214, 210, 230, 255))
    save(up2(f), "hud/bar_frame.png")

    def bar(base):
        b = Image.new("RGBA", (280, 12), (0, 0, 0, 0))
        d = ImageDraw.Draw(b)
        for y in range(12):
            if y == 0:
                c = mix(base, (255, 255, 255), 0.55)
            elif y < 4:
                c = mix(base, (255, 255, 255), 0.2)
            elif y < 9:
                c = base
            else:
                c = mix(base, (30, 10, 30), 0.35)
            d.line([(0, y), (279, y)], fill=(*c, 255))
        return up2(b)

    save(bar(HEALTH), "hud/bar_fill.png")
    save(bar(DAMAGE), "hud/bar_damage.png")

    for name, fill in (("win_pip_empty", None), ("win_pip_full", HEALTH)):
        p = Image.new("RGBA", (12, 12), (0, 0, 0, 0))
        d = ImageDraw.Draw(p)
        d.polygon([(6, 0), (11, 5), (11, 6), (6, 11), (5, 11), (0, 6), (0, 5), (5, 0)], fill=(*INK, 255))
        d.polygon([(6, 2), (9, 5), (9, 6), (6, 9), (5, 9), (2, 6), (2, 5), (5, 2)], fill=(*(fill or (58, 52, 76)), 255))
        if fill:
            d.point((5, 4), fill=(255, 255, 255, 255))
        save(up2(p), f"hud/{name}.png")

    for tag, color in (("p1", P1_COLOR), ("p2", P2_COLOR)):
        t = styled_text(tag.upper(), 3, mix(color, (255, 255, 255), 0.3), color, border=2, shadow=2)
        cell = Image.new("RGBA", (64, 32), (0, 0, 0, 0))
        cell.alpha_composite(t, ((64 - t.width) // 2, (32 - t.height) // 2))
        save(cell, f"hud/{tag}_tag.png")

    silver = ((255, 255, 255), (150, 158, 196))
    banners = {
        "banner_round_1": ("ROUND 1", 12, silver),
        "banner_round_2": ("ROUND 2", 12, silver),
        "banner_round_3": ("ROUND 3", 12, silver),
        "banner_ready": ("READY", 14, silver),
        "banner_fight": ("FIGHT!", 18, (mix(ACCENT, (255, 255, 255), 0.4), (70, 168, 60))),
        "banner_ko": ("K.O.", 16, ((255, 236, 140), (232, 86, 64))),
        "banner_double_ko": ("DOUBLE K.O.", 10, ((255, 236, 140), (232, 86, 64))),
        "banner_p1_wins": ("P1 WINS", 14, (mix(P1_COLOR, (255, 255, 255), 0.5), (30, 130, 170))),
        "banner_p2_wins": ("P2 WINS", 14, (mix(P2_COLOR, (255, 255, 255), 0.5), (190, 100, 30))),
    }
    for name, (text, scale, (top, bottom)) in banners.items():
        img = styled_text(text, scale, top, bottom, border=max(4, scale // 3), shadow=max(4, scale // 2))
        save(fit(img, 800, 200), f"hud/{name}.png")

    tb = Image.new("RGBA", (48, 32), (0, 0, 0, 0))
    d = ImageDraw.Draw(tb)
    d.polygon([(0, 0), (47, 0), (41, 31), (6, 31)], fill=(*INK, 255))
    d.polygon([(2, 1), (45, 1), (40, 29), (7, 29)], fill=(36, 30, 48, 255))
    save(up2(tb), "hud/timer_bg.png")


# ---------------------------------------------------------------- menus


def gen_ui():
    # Background: the stage sky, darker, with the two fighters as faint
    # "ghost frames" on the right; the left side stays plain for text.
    sky = dither_gradient(640, 360, [(0.0, (12, 10, 26)), (0.6, (30, 24, 56)), (1.0, (54, 40, 78))], steps=10)
    bg = Image.fromarray(sky, "RGB").convert("RGBA")
    d = ImageDraw.Draw(bg)
    rng = random.Random(2)
    for _ in range(120):
        x, y = rng.randrange(320, 640), rng.randrange(0, 260)
        d.point((x, y), fill=mix((50, 46, 84), (170, 170, 210), rng.random() ** 3))
    for r, c in ((60, (34, 28, 62)), (46, (44, 36, 76)), (34, (64, 54, 100))):
        d.ellipse([500 - r, 120 - r, 500 + r, 120 + r], fill=c)
    d.rectangle([0, 300, 640, 360], fill=(26, 20, 36))
    d.line([(0, 300), (640, 300)], fill=(84, 70, 96))
    big = bg.resize((1280, 720), Image.NEAREST)
    # P1 lunging with two fading afterimages, P2 reeling from the hit.
    ghosts = [
        (fighter.light()[0], "p1", 640, 0.16, False),
        (fighter.light()[1], "p1", 700, 0.3, False),
        (fighter.light()[2], "p1", 760, 1.0, False),
        (fighter.hit_high()[0], "p2", 1090, 1.0, True),
    ]
    for pose, scheme, x, alpha, flip in ghosts:
        spr = fighter.draw_pose(pose, scheme).resize((640, 512), Image.NEAREST)
        if flip:
            spr = spr.transpose(Image.FLIP_LEFT_RIGHT)
        a = np.array(spr)
        a[..., 3] = (a[..., 3] * alpha).astype(np.uint8)
        big.alpha_composite(Image.fromarray(a, "RGBA"), (x - 320, 600 - 480))
    # Darken the text side with a dithered fade.
    arr = np.array(big).astype(np.float32)
    for x in range(0, 700):
        k = 0.55 * (1 - x / 700)
        arr[:, x, :3] *= 1 - k
    save(Image.fromarray(arr.astype(np.uint8), "RGBA").convert("RGB").convert("RGBA"), "ui/menu_bg.png")

    # Logo: the name with two afterimages trailing it (a ghost frame).
    main = styled_text("GHOSTFRAME", 9, (255, 255, 255), (170, 176, 214), border=4, shadow=5)
    logo = Image.new("RGBA", (main.width + 24, main.height + 4), (0, 0, 0, 0))
    for off, color, alpha in ((0, P2_COLOR, 0.35), (12, P1_COLOR, 0.55)):
        echo = styled_text("GHOSTFRAME", 9, color, color, outline=INK, border=1, shadow=0, gloss=False)
        a = np.array(echo)
        a[..., 3] = (a[..., 3] * alpha).astype(np.uint8)
        logo.alpha_composite(Image.fromarray(a, "RGBA"), (off, 6))
    logo.alpha_composite(main, (24, 0))
    save(fit(logo, 640, 200), "ui/logo.png")

    c = Image.new("RGBA", (16, 16), (0, 0, 0, 0))
    d = ImageDraw.Draw(c)
    d.polygon([(2, 2), (14, 8), (2, 14), (5, 8)], fill=(*INK, 255))
    d.polygon([(3, 4), (12, 8), (3, 12), (6, 8)], fill=(*ACCENT, 255))
    d.line([(4, 5), (9, 7)], fill=(230, 255, 210, 255))
    save(up2(c), "ui/cursor.png")

    p = Image.new("RGBA", (400, 160), (0, 0, 0, 0))
    d = ImageDraw.Draw(p)
    cut = 6
    outer = [(cut, 0), (399 - cut, 0), (399, cut), (399, 159 - cut), (399 - cut, 159), (cut, 159), (0, 159 - cut), (0, cut)]
    d.polygon(outer, fill=(*INK, 240))
    inner = [(cut, 2), (397 - cut, 2), (397, cut), (397, 157 - cut), (397 - cut, 157), (cut, 157), (2, 157 - cut), (2, cut)]
    d.polygon(inner, outline=(106, 98, 140, 255), fill=(26, 22, 40, 228))
    d.line([(cut + 1, 3), (396 - cut, 3)], fill=(150, 144, 186, 255))
    for x, y in ((5, 5), (394, 5), (5, 154), (394, 154)):
        d.point((x, y), fill=(*ACCENT, 255))
    save(up2(p), "ui/panel.png")

    for name, border, glow in (("code_slot", (106, 98, 140), None), ("code_slot_active", ACCENT, ACCENT)):
        s = Image.new("RGBA", (36, 48), (0, 0, 0, 0))
        d = ImageDraw.Draw(s)
        d.rectangle([0, 0, 35, 47], fill=(*INK, 255))
        d.rectangle([1, 1, 34, 46], fill=(26, 22, 40, 255), outline=(*border, 255))
        d.rectangle([6, 40, 29, 41], fill=(*(glow or (90, 84, 118)), 255))
        if glow:
            d.rectangle([2, 2, 33, 2], fill=(*mix(glow, (255, 255, 255), 0.4), 255))
        save(up2(s), f"ui/{name}.png")

    def keycap(label, w):
        k = Image.new("RGBA", (w, 24), (0, 0, 0, 0))
        d = ImageDraw.Draw(k)
        d.rounded_rectangle([0, 0, w - 1, 23], 3, fill=(*INK, 255))
        d.rounded_rectangle([1, 1, w - 2, 22], 3, fill=(120, 124, 150, 255))
        d.rounded_rectangle([1, 1, w - 2, 18], 3, fill=(214, 218, 232, 255))
        d.line([(3, 2), (w - 4, 2)], fill=(250, 250, 255, 255))
        ink = (40, 36, 56, 255)
        if label in ("up", "down", "left", "right"):
            cx, cy = w // 2, 10
            tri = {
                "up": [(cx, cy - 4), (cx + 4, cy + 2), (cx - 4, cy + 2)],
                "down": [(cx, cy + 4), (cx + 4, cy - 2), (cx - 4, cy - 2)],
                "left": [(cx - 4, cy), (cx + 2, cy - 4), (cx + 2, cy + 4)],
                "right": [(cx + 4, cy), (cx - 2, cy - 4), (cx - 2, cy + 4)],
            }[label]
            d.polygon(tri, fill=ink)
        else:
            text = label.upper()
            tw = pixfont.text_width(text)
            ox, oy = (w - tw) // 2, 6
            for x, y in pixfont.text_pixels(text):
                d.point((ox + x, oy + y), fill=ink)
        return up2(k)

    for k in ["a", "d", "w", "s", "f", "g", "k", "l", "1", "2", "3", "4", "up", "down", "left", "right"]:
        save(keycap(k, 24), f"ui/keys/{k}.png")
    save(keycap("enter", 48), "ui/keys/enter.png")
    save(keycap("esc", 48), "ui/keys/esc.png")

    for i in range(8):
        s = Image.new("RGBA", (32, 32), (0, 0, 0, 0))
        d = ImageDraw.Draw(s)
        for k in range(8):
            a = k * math.pi / 4 - math.pi / 2
            x, y = 16 + math.cos(a) * 11, 16 + math.sin(a) * 11
            age = (i - k) % 8
            r = 3 if age == 0 else 2.5 if age == 1 else 2
            col = ACCENT if age == 0 else mix(ACCENT, (40, 36, 60), min(1.0, age / 4))
            d.ellipse([x - r, y - r, x + r, y + r], fill=(*col, 255))
        save(up2(s), f"ui/spinner/{i:02}.png")


# ---------------------------------------------------------------- font


def gen_font():
    from fontTools.fontBuilder import FontBuilder
    from fontTools.pens.ttGlyphPen import TTGlyphPen

    unit = 100  # font units per art pixel; 1000 per em = 10 pixels
    chars = sorted(pixfont.GLYPHS)
    names = {ch: ("space" if ch == " " else f"uni{ord(ch):04X}") for ch in chars}
    order = [".notdef"] + [names[ch] for ch in chars]
    fb = FontBuilder(1000, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap({ord(ch): names[ch] for ch in chars})

    def draw(rows):
        pen = TTGlyphPen(None)
        for r, row in enumerate(rows):
            for c, px in enumerate(row):
                if px != "#":
                    continue
                x0, x1 = c * unit, (c + 1) * unit
                y1 = (pixfont.CAP_ROWS - r) * unit
                y0 = y1 - unit
                # Clockwise outer contour (TrueType convention).
                pen.moveTo((x0, y0))
                pen.lineTo((x0, y1))
                pen.lineTo((x1, y1))
                pen.lineTo((x1, y0))
                pen.closePath()
        return pen.glyph()

    glyphs = {".notdef": draw(["#####", "#...#", "#...#", "#...#", "#...#", "#...#", "#####"])}
    for ch in chars:
        glyphs[names[ch]] = draw(pixfont.GLYPHS[ch])
    fb.setupGlyf(glyphs)
    adv = pixfont.ADVANCE * unit
    fb.setupHorizontalMetrics({n: (adv, 0) for n in order})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({"familyName": "GhostFrame Pixel", "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
    fb.setupPost()
    fb.save(out_path("font/main.ttf"))


def main():
    gen_fighter()
    gen_stage()
    gen_fx()
    gen_hud()
    gen_ui()
    gen_font()
    print("wrote", OUT)


if __name__ == "__main__":
    main()
