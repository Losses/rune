"""Make CRT Wall's textures: python3 make_textures.py

Needs Pillow and NumPy. Writes ../assets/plastic.png, 512 x 512, tiling, 0.4 m across: the
cabinets' moulded plastic. Data, not colour (the scene loads it with linear = true):
  R, G  the normal's x and y, 0.5 for flat (z is worked out in the shader),
  B     how open each spot is, 1 on the face, darker in the scratches and the stipple's pits,
  A     the face's tone, 0.5 for even: lighter where dust settled and along scratches.
"""
import math
import pathlib

import numpy as np
from PIL import Image

OUT = pathlib.Path(__file__).resolve().parent.parent / "assets"
rng = np.random.default_rng(11)


# ---- Plastic ----

SIZE = 512
METRES = 0.4
PX = SIZE / METRES


def smooth_noise(cutoff, power=2.0):
    """Tiling noise from white noise filtered in frequency, scaled to -1..1."""
    f = np.fft.fftfreq(SIZE)
    r = np.hypot(*np.meshgrid(f, f))
    spectrum = np.fft.fft2(rng.standard_normal((SIZE, SIZE))) * np.exp(-((r / cutoff) ** power))
    n = np.real(np.fft.ifft2(spectrum))
    return n / np.abs(n).max()


def plastic():
    y, x = np.mgrid[0:SIZE, 0:SIZE].astype(np.float64)
    # Heights in metres. The mould's stipple: fine, even pits a few tenths of a millimetre deep,
    # with a faint swell where the plastic cooled unevenly.
    stipple = smooth_noise(0.25, 4.0)
    height = 0.00012 * stipple + 0.0003 * smooth_noise(0.008)
    open_ = 1.0 - 0.12 * np.clip(-stipple, 0.0, 1.0)
    tone = 0.5 + 0.1 * smooth_noise(0.004) + 0.04 * smooth_noise(0.03)

    # Scratches: thin straight grooves, most of them short and along the face, a few long.
    for _ in range(26):
        cx, cy = rng.uniform(0, SIZE, 2)
        a = rng.normal(0.0, 0.35) + (math.pi / 2 if rng.random() < 0.3 else 0.0)
        length = rng.uniform(0.01, 0.06 if rng.random() < 0.8 else 0.16) * PX
        width = rng.uniform(0.6, 1.4)
        ux, uy = math.cos(a), math.sin(a)
        dx = (x - cx + SIZE / 2) % SIZE - SIZE / 2
        dy = (y - cy + SIZE / 2) % SIZE - SIZE / 2
        along = dx * ux + dy * uy
        across = -dx * uy + dy * ux
        inside = np.clip(1.0 - np.abs(along) / (length / 2), 0.0, 1.0) ** 0.3
        groove = np.exp(-((across / width) ** 2)) * inside
        height -= 0.00004 * groove
        open_ -= 0.25 * groove
        tone += 0.12 * groove

    gx = (np.roll(height, -1, axis=1) - np.roll(height, 1, axis=1)) * PX / 2
    gy = (np.roll(height, -1, axis=0) - np.roll(height, 1, axis=0)) * PX / 2
    n = np.stack([-gx, -gy, np.ones_like(gx)], axis=-1)
    n /= np.linalg.norm(n, axis=-1, keepdims=True)
    rgba = np.stack(
        [n[..., 0] * 0.5 + 0.5, n[..., 1] * 0.5 + 0.5, np.clip(open_, 0.0, 1.0), np.clip(tone, 0.0, 1.0)],
        axis=-1,
    )
    Image.fromarray(np.round(rgba * 255).astype(np.uint8), "RGBA").save(OUT / "plastic.png")


def main():
    OUT.mkdir(exist_ok=True)
    plastic()
    print(OUT)


if __name__ == "__main__":
    main()
