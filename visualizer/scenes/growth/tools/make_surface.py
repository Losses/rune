"""Make the plates' surface: python make_surface.py

Needs Pillow and NumPy. Writes ../assets/surface.png, 512 x 512, tiling, one 1.6 m cell across:
cast concrete, four tie holes, pores and a faint swell. No formwork seam: the texture is laid
across a plate several cells wide, and a seam at each cell's edge would cut it back into cells.

The channels are data (the scene loads it with linear = true):
  R, G  the normal's x and y, 0.5 for flat (z is worked out in the shader),
  B     how open each spot is, 1 on the face and darker in the holes and pores,
  A     the face's own tone, 0.5 for even, lighter and darker where the pour came out so.
"""
import pathlib

import numpy as np
from PIL import Image

SIZE = 512
METRES = 1.6
PX = SIZE / METRES
rng = np.random.default_rng(7)


def smooth_noise(cutoff, power=2.0):
    """Tiling noise from white noise filtered in frequency, scaled to -1..1."""
    f = np.fft.fftfreq(SIZE)
    r = np.hypot(*np.meshgrid(f, f))
    spectrum = np.fft.fft2(rng.standard_normal((SIZE, SIZE))) * np.exp(-((r / cutoff) ** power))
    n = np.real(np.fft.ifft2(spectrum))
    return n / np.abs(n).max()


def wrapped_distance(x, y, cx, cy):
    dx = (x - cx + SIZE / 2) % SIZE - SIZE / 2
    dy = (y - cy + SIZE / 2) % SIZE - SIZE / 2
    return np.hypot(dx, dy)


def main():
    y, x = np.mgrid[0:SIZE, 0:SIZE].astype(np.float64)
    # Heights in metres.
    height = 0.0015 * smooth_noise(0.01) + 0.0004 * smooth_noise(0.06)
    open_ = np.ones((SIZE, SIZE))

    # Tie holes, 2 x 2, cones 3 cm across and 1.5 cm deep.
    for cx in (0.4, 1.2):
        for cy in (0.4, 1.2):
            d = wrapped_distance(x, y, cx * PX, cy * PX) / PX
            cone = np.clip(1.0 - d / 0.015, 0.0, 1.0)
            height -= 0.015 * cone
            open_ -= 0.55 * np.clip(cone * 1.6, 0.0, 1.0)

    # Pores: small pits, a few millimetres, scattered.
    for _ in range(900):
        cx, cy = rng.uniform(0, SIZE, 2)
        r = rng.uniform(0.0008, 0.003)
        d = wrapped_distance(x, y, cx, cy) / PX
        pit = np.clip(1.0 - (d / r) ** 2, 0.0, 1.0)
        height -= r * 0.6 * pit
        open_ -= 0.5 * pit

    # The normal from the height's slope, wrapping at the edges so it tiles.
    gx = (np.roll(height, -1, axis=1) - np.roll(height, 1, axis=1)) * PX / 2
    gy = (np.roll(height, -1, axis=0) - np.roll(height, 1, axis=0)) * PX / 2
    n = np.stack([-gx, -gy, np.ones_like(gx)], axis=-1)
    n /= np.linalg.norm(n, axis=-1, keepdims=True)

    # The pour's tone: broad clouds and a few streaks down the face.
    tone = 0.5 + 0.18 * smooth_noise(0.006) + 0.06 * smooth_noise(0.04)
    tone += 0.04 * smooth_noise(0.05)[:1, :] * np.ones((SIZE, 1))

    rgba = np.stack(
        [n[..., 0] * 0.5 + 0.5, n[..., 1] * 0.5 + 0.5, np.clip(open_, 0.0, 1.0), np.clip(tone, 0.0, 1.0)],
        axis=-1,
    )
    out = pathlib.Path(__file__).resolve().parent.parent / "assets" / "surface.png"
    out.parent.mkdir(exist_ok=True)
    Image.fromarray(np.round(rgba * 255).astype(np.uint8), "RGBA").save(out)
    print(out)


if __name__ == "__main__":
    main()
