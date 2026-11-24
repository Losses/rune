"""Make the badges' crests: python make_crests.py

Needs Pillow, NumPy and SciPy. Writes ../assets/crests.png, 1024 x 1024: sixteen cells of 256
pixels in a 4 x 4 grid, each a Japanese family crest (kamon) drawn in the round, as a signed
distance field: the grey is 0.5 on the crest's edge, brighter inside and darker outside, SPREAD
cell pixels to full white or black. The scene loads it as data (linear = true) and draws the
crest's edge sharp at any size, in gold leaf.

Each crest is drawn in a unit circle (its reach is 1), at 8 times the cell's size, and its
distance worked out there. Cells left over are empty.

The crests (by cell): 0 mitsudomoe (three tomoe), 1 hanabishi (flower lozenge), 2 asanoha (hemp
leaf), 3 kagome (woven hexagram), 4 shippo (seven treasures), 5 umebachi (plum bowl),
6 seigaiha (waves), 7 mitsumori kikko (three tortoiseshells), 8 kuyo (nine stars), 9 maru ni
mitsubiki (three bars in a ring), 10 futatsudomoe (two tomoe), 11 yotsume bishi (four
lozenges). The scene keeps, for each, how far it turns before it looks the same again.
"""
import math
import pathlib

import numpy as np
from PIL import Image, ImageDraw
from scipy.ndimage import distance_transform_edt

CELL = 256
GRID = 4
SUPER = 8
SPREAD = 16.0
# How much of the cell's half the crest's unit circle reaches (the rest is room for the field).
REACH = 0.86


class Crest:
    """Drawing in the crest's own coordinates: x right, y up, the unit circle its reach."""

    def __init__(self):
        self.n = CELL * SUPER
        self.img = Image.new("L", (self.n, self.n), 0)
        self.d = ImageDraw.Draw(self.img)

    def p(self, x, y):
        s = self.n / 2 * REACH
        return (self.n / 2 + x * s, self.n / 2 - y * s)

    def scale(self, r):
        return r * self.n / 2 * REACH

    def disk(self, x, y, r, ink=255):
        cx, cy = self.p(x, y)
        rr = self.scale(r)
        self.d.ellipse([cx - rr, cy - rr, cx + rr, cy + rr], fill=ink)

    def ring(self, x, y, r, w, ink=255):
        """A circle's line, drawn without touching what is inside it."""
        cx, cy = self.p(x, y)
        ro = self.scale(r + w / 2)
        self.d.ellipse([cx - ro, cy - ro, cx + ro, cy + ro], outline=ink, width=max(int(round(self.scale(w))), 1))

    def clip(self, r):
        """Clear everything outside the circle of radius r."""
        keep = Image.new("L", self.img.size, 0)
        ImageDraw.Draw(keep).ellipse([self.p(-r, r), self.p(r, -r)], fill=255)
        self.img.paste(0, mask=Image.eval(keep, lambda v: 255 - v))

    def poly(self, pts, ink=255):
        self.d.polygon([self.p(x, y) for x, y in pts], fill=ink)

    def line(self, a, b, w, ink=255):
        (ax, ay), (bx, by) = a, b
        dx, dy = bx - ax, by - ay
        L = math.hypot(dx, dy)
        nx, ny = -dy / L * w / 2, dx / L * w / 2
        self.poly([(ax + nx, ay + ny), (bx + nx, by + ny), (bx - nx, by - ny), (ax - nx, ay - ny)], ink)
        self.disk(ax, ay, w / 2, ink)
        self.disk(bx, by, w / 2, ink)

    def outline(self, pts, w, ink=255):
        for i in range(len(pts)):
            self.line(pts[i], pts[(i + 1) % len(pts)], w, ink)

    def mask(self):
        return np.asarray(self.img) > 127


def polar(r, deg):
    a = math.radians(deg)
    return (r * math.cos(a), r * math.sin(a))


def hexagon(r, start=0.0, cx=0.0, cy=0.0):
    return [(cx + polar(r, start + 60 * k)[0], cy + polar(r, start + 60 * k)[1]) for k in range(6)]


def tomoe(c, head_deg, count):
    """A tomoe: a round head, and a disk swept from it along a spiral out towards the rim and round
    behind it, shrinking to the tail's point."""
    sweep = 360.0 / count * (0.82 if count == 3 else 0.9)
    head_r, head_at = (0.29, 0.5) if count == 3 else (0.46, 0.46)
    steps = 200
    for i in range(steps + 1):
        u = i / steps
        at = head_at + (0.97 - head_at) * u ** 1.6
        r = head_r * (1.0 - u) ** 1.35
        r = min(r, 0.97 - at)
        if r > 0.002:
            c.disk(*polar(at, head_deg - u * sweep), r)
    c.disk(*polar(head_at, head_deg), head_r)


def mitsudomoe(c):
    for k in range(3):
        tomoe(c, 90 + 120 * k, 3)


def futatsudomoe(c):
    for k in range(2):
        tomoe(c, 90 + 180 * k, 2)


def hanabishi(c):
    """Four heart-shaped petals in a lozenge, wider than tall."""
    for deg, length in ((0, 0.98), (90, 0.74), (180, 0.98), (270, 0.74)):
        ux, uy = polar(1, deg)
        vx, vy = -uy, ux
        w = 0.3 * length
        tip = 0.05
        lobe = length - w
        for side in (-1, 1):
            c.disk(ux * lobe + vx * side * w * 0.75, uy * lobe + vy * side * w * 0.75, w)
        c.poly([(ux * tip, uy * tip),
                (ux * lobe + vx * w * 1.62, uy * lobe + vy * w * 1.62),
                (ux * lobe - vx * w * 1.62, uy * lobe - vy * w * 1.62)])
        # The notch at each petal's tip.
        c.poly([(ux * (length - w * 0.55), uy * (length - w * 0.55)),
                (ux * 1.2 + vx * w * 0.5, uy * 1.2 + vy * w * 0.5),
                (ux * 1.2 - vx * w * 0.5, uy * 1.2 - vy * w * 0.5)], 0)


def asanoha(c):
    """The hemp leaf: a hexagon cut into six triangles, each drawn to its middle."""
    w = 0.07
    vs = hexagon(0.97, 90)
    c.outline(vs, w)
    for i in range(6):
        a, b = vs[i], vs[(i + 1) % 6]
        g = ((a[0] + b[0]) / 3, (a[1] + b[1]) / 3)
        c.line((0, 0), a, w)
        for e in ((0, 0), a, b):
            c.line(g, e, w * 0.8)


def kagome(c):
    """Two triangles woven over and under each other: at the six crossings, by turns."""
    w = 0.12
    up = [polar(0.97, 90 + 120 * k) for k in range(3)]
    down = [polar(0.97, 270 + 120 * k) for k in range(3)]
    c.outline(up, w)
    c.outline(down, w)

    def crossing(a, b, p, q):
        # Where segment ab meets segment pq.
        d1 = (b[0] - a[0], b[1] - a[1])
        d2 = (q[0] - p[0], q[1] - p[1])
        den = d1[0] * d2[1] - d1[1] * d2[0]
        t = ((p[0] - a[0]) * d2[1] - (p[1] - a[1]) * d2[0]) / den
        u = ((p[0] - a[0]) * d1[1] - (p[1] - a[1]) * d1[0]) / den
        if 0 < t < 1 and 0 < u < 1:
            return (a[0] + d1[0] * t, a[1] + d1[1] * t)
        return None

    found = []
    for i in range(3):
        for j in range(3):
            a, b = up[i], up[(i + 1) % 3]
            p, q = down[j], down[(j + 1) % 3]
            x = crossing(a, b, p, q)
            if x:
                found.append((math.atan2(x[1], x[0]), x, (a, b), (p, q)))
    found.sort()
    for k, (_, x, upper, lower) in enumerate(found):
        top = upper if k % 2 == 0 else lower
        (a, b) = top
        L = math.hypot(b[0] - a[0], b[1] - a[1])
        ux, uy = (b[0] - a[0]) / L, (b[1] - a[1]) / L
        reach = w * 1.6
        e0 = (x[0] - ux * reach, x[1] - uy * reach)
        e1 = (x[0] + ux * reach, x[1] + uy * reach)
        # A dark band either side of the line on top, then the line again over it.
        c.line(e0, e1, w * 2.3, 0)
        c.line(e0, e1, w)


def shippo(c):
    """A ring with four circles overlapping in it, their lenses making a four-petalled flower."""
    w = 0.08
    for k in range(4):
        c.ring(*polar(0.46, 45 + 90 * k), 0.65, w)
    c.clip(0.92)
    c.ring(0, 0, 0.92, w)


def umebachi(c):
    """The plum bowl: five round petals about a round middle, each petal pierced."""
    for k in range(5):
        x, y = polar(0.6, 90 + 72 * k)
        c.disk(x, y, 0.37)
    for k in range(5):
        x, y = polar(0.6, 90 + 72 * k)
        c.disk(x, y, 0.12, 0)
    c.disk(0, 0, 0.25, 0)
    c.disk(0, 0, 0.17)


def seigaiha(c):
    """Waves: rows of fans of rings, each over the one behind, within a ring."""
    rows = [(-0.62, [-0.62, 0.0, 0.62]), (-0.3, [-0.93, -0.31, 0.31, 0.93]), (0.02, [-0.62, 0.0, 0.62]), (0.34, [-0.31, 0.31]), (0.66, [0.0])]
    for y, xs in reversed(rows):
        for x in xs:
            c.disk(x, y, 0.35, 0)
            for r in (0.31, 0.21, 0.11):
                c.ring(x, y, r, 0.055)
    c.clip(0.88)
    c.ring(0, 0, 0.93, 0.08)


def mitsumori_kikko(c):
    """Three tortoiseshells heaped one on two, each touching the others, each a double hexagon."""
    r = 0.5
    a = math.sqrt(3) / 2 * r
    for cx, cy in ((0, 0.75 * r), (-a, -0.75 * r), (a, -0.75 * r)):
        c.outline(hexagon(r * 0.95, 30, cx, cy), 0.075)
        c.outline(hexagon(r * 0.6, 30, cx, cy), 0.045)


def kuyo(c):
    """Nine stars: a big one in the middle, eight round it."""
    c.disk(0, 0, 0.3)
    for k in range(8):
        c.disk(*polar(0.72, 90 + 45 * k), 0.2)


def mitsubiki(c):
    """Three bars in a ring."""
    c.ring(0, 0, 0.9, 0.12)
    for y in (-0.36, 0.0, 0.36):
        half = math.sqrt(max(0.84 ** 2 - y ** 2, 0)) - 0.06
        c.poly([(-half, y + 0.11), (half, y + 0.11), (half, y - 0.11), (-half, y - 0.11)])


def yotsume_bishi(c):
    """Four lozenges in a lozenge, each with a lozenge cut out."""
    for x, y in ((0, 0.5), (0.5, 0), (0, -0.5), (-0.5, 0)):
        c.poly([(x, y + 0.46), (x + 0.46, y), (x, y - 0.46), (x - 0.46, y)])
        c.poly([(x, y + 0.2), (x + 0.2, y), (x, y - 0.2), (x - 0.2, y)], 0)


CRESTS = [mitsudomoe, hanabishi, asanoha, kagome, shippo, umebachi, seigaiha, mitsumori_kikko, kuyo, mitsubiki, futatsudomoe, yotsume_bishi]


def field(mask):
    """The signed distance of each pixel to the edge, in cell pixels, positive inside."""
    inside = distance_transform_edt(mask)
    outside = distance_transform_edt(~mask)
    d = (inside - outside) / SUPER
    # Down to the cell's size: the mean of each SUPER x SUPER block.
    n = mask.shape[0] // SUPER
    return d.reshape(n, SUPER, n, SUPER).mean(axis=(1, 3))


def main():
    atlas = np.zeros((CELL * GRID, CELL * GRID))
    for i, draw in enumerate(CRESTS):
        c = Crest()
        draw(c)
        d = field(c.mask())
        y, x = divmod(i, GRID)
        atlas[y * CELL:(y + 1) * CELL, x * CELL:(x + 1) * CELL] = np.clip(0.5 + d / (2 * SPREAD), 0, 1)
    out = pathlib.Path(__file__).resolve().parent.parent / "assets" / "crests.png"
    out.parent.mkdir(exist_ok=True)
    Image.fromarray(np.round(atlas * 255).astype(np.uint8), "L").save(out)
    print(out)


if __name__ == "__main__":
    main()
