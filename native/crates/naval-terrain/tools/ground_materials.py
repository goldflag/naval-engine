#!/usr/bin/env python3
"""The land's tiling ground materials (`naval_terrain::ground_materials`), made from Poly Haven's CC0 textures.

    python3 native/crates/naval-terrain/tools/ground_materials.py [--only grass,sand]

Downloads each material's 1K colour, OpenGL normal and displacement maps through Poly Haven's public API (cached in
`.build/ground-materials/src/`, curl), and writes two JPEGs a material into `public/terrain/materials/`, seamless as the
sources are (512² or 1024², `MATERIALS`):

- `<name>-albedo.jpg`: the colour as a ratio to its own local mean, in linear light, halved (128 is the mean). The land
  multiplies its own colour (the satellite photograph, a style's palette) by it, so the material adds structure and
  never its colour: everything wider than about a third of the tile is taken out (each texel over a Gaussian blur of
  the tile, wrapping), the chroma kept at `CHROMA` of its variation, the contrast held between `MIN_CONTRAST` and
  `MAX_CONTRAST` (the standard deviation of the luminance's logarithm), clamped and renormalised to a mean of exactly 1.
  The forest floor's leaves and needles keep more contrast and their own chroma, squeezed toward 1 to fit the bytes
  (`LOOKS`).
- `<name>-surface.jpg`: red and green the normal map's x and y (OpenGL: green toward the image's top), blue the height
  for the height-based blend between materials (high-passed like the colour, its 1st and 99th percentiles at 0.05 and
  0.95).

Both 4:4:4 (no chroma subsampling: the channels are data). Needs Python 3 with Pillow and NumPy, and curl.
"""

import argparse
import io
import json
import os
import subprocess
import sys

import numpy as np
from PIL import Image

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../.."))
CACHE = os.path.join(ROOT, ".build/ground-materials/src")
OUT = os.path.join(ROOT, "public/terrain/materials")
SIZE = 1024
QUALITY = 85

# The order is the texture array's (naval_terrain::ground_materials::GROUND_MATERIALS holds the same names).
# Each written at `side` texels: 512 for the close-up photographs repeating every 5-10 m (their 1K texels, under a
# centimetre, are never resolved from 30 m), 1024 for the aerial ones repeating every 12-30 m and for the two seen from
# an eye at a man's height (the forest floor and the fine sand, repeating every 2-3 m: a few millimetres a texel, as the
# nearest ground's pixels are; the pack holds every layer at 1024² either way).
MATERIALS = [
    ("grass", "grass_ground", 512),
    ("dry-grass", "withered_grass", 512),
    ("soil", "farm_soil", 512),
    ("scrub", "sparse_grass", 512),
    ("dry-scrub", "aerial_rocks_01", 1024),
    ("forest", "leaves_forest_ground", 1024),
    ("rock", "aerial_rocks_02", 1024),
    ("cliff", "marble_cliff_05", 1024),
    ("sand", "coast_sand_01", 1024),
    ("wet-sand", "coast_sand_02", 1024),
    ("shingle", "clean_pebbles", 512),
    ("snow", "snow_02", 512),
    ("mud", "brown_mud_02", 512),
    ("paving", "aerial_asphalt_01", 1024),
    ("fine-sand", "sand_03", 1024),
    # The cliffs' rock by region (`naval_terrain::ground_materials::CliffRock`): photographed face on, read on the
    # land's walls with up kept up (`materials.wgsl`, `cliff_read`).
    ("limestone", "marble_cliff_04", 1024),
    ("basalt", "dark_rock_02", 1024),
    ("granite", "marble_rock_01", 1024),
    # The forest floor's needles under a conifer wood (`naval_terrain::ground_materials::needle_floor`), read through the
    # forest floor's weight where its trees are conifers (`materials.wgsl`, `floor_read`), seen from a man's height.
    ("needles", "forrest_ground_03", 1024),
]

CHROMA = 0.4
MIN_CONTRAST = 0.10
MAX_CONTRAST = 0.32
# The materials seen from a man's height, whose colour keeps more of the photograph's contrast: the forest floor's leaf
# litter, whose leaves are told from the dark gaps between them by it (most of the photograph's 0.86: held at 0.32 they
# read as gravel), and its needles (their own 0.42). Their chroma: the litter's 45% (at 70% its autumn beech leaves read
# as orange wood chips in the sun; the litter's own colour is the land's, `surface.wgsl`'s `litter_color`), the needles'
# less, greyer. Their chroma, their most contrast, and how far their ratio is squeezed toward 1 to fit the bytes (its
# departures from 1 over `squeeze`, so ratios up to 1 + 0.95 squeeze are kept; naval_terrain::ground_materials's
# `squeezed` holds the same and the land's uniforms multiply them back).
LOOKS = {
    "forest": {"chroma_kept": 0.45, "max_contrast": 0.75, "squeeze": 2.5},
    "needles": {"chroma_kept": 0.35, "max_contrast": 0.6, "squeeze": 2.0},
}
# The blur's standard deviation, in tiles: what is wider goes.
HIGH_PASS = 1.0 / 6.0


def fetch(polyhaven_id):
    """The 1K diffuse, OpenGL normal and displacement maps, from the cache or Poly Haven."""
    folder = os.path.join(CACHE, polyhaven_id)
    os.makedirs(folder, exist_ok=True)
    listing = None
    paths = {}
    for key, name in (("Diffuse", "diff"), ("nor_gl", "nor"), ("Displacement", "disp")):
        found = [f for f in os.listdir(folder) if f.startswith(name + ".")]
        if found:
            paths[name] = os.path.join(folder, found[0])
            continue
        if listing is None:
            listing = json.loads(
                subprocess.check_output(["curl", "-sf", f"https://api.polyhaven.com/files/{polyhaven_id}"])
            )
        files = listing[key]["1k"]
        url = (files.get("jpg") or files.get("png"))["url"]
        path = os.path.join(folder, name + "." + url.rsplit(".", 1)[1])
        subprocess.check_call(["curl", "-sf", "-o", path, url])
        paths[name] = path
    return paths


def load(path, mode):
    image = Image.open(path).convert(mode)
    if image.size != (SIZE, SIZE):
        image = image.resize((SIZE, SIZE), Image.LANCZOS)
    return np.asarray(image, dtype=np.float64) / 255.0


def srgb_to_linear(c):
    return np.where(c < 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def blur(values, sigma_tiles):
    """A Gaussian blur over the tile, wrapping at its edges (the tile repeats), by its spectrum."""
    n = values.shape[0]
    f = np.fft.fftfreq(n)
    sigma = sigma_tiles * n
    kernel = np.exp(-2.0 * (np.pi * sigma) ** 2 * (f[:, None] ** 2 + f[None, :] ** 2))
    if values.ndim == 2:
        return np.real(np.fft.ifft2(np.fft.fft2(values) * kernel))
    return np.stack([blur(values[..., c], sigma_tiles) for c in range(values.shape[2])], axis=-1)


def albedo_ratio(diffuse, chroma_kept=CHROMA, max_contrast=MAX_CONTRAST, squeeze=1.0):
    linear = srgb_to_linear(diffuse)
    luminance = linear @ np.array([0.2126, 0.7152, 0.0722])
    local = np.maximum(blur(luminance, HIGH_PASS), 1e-4)
    ratio = np.maximum(luminance, 1e-4) / local
    # The contrast, in the logarithm, held within its bounds about the tile's own mean.
    log = np.log(ratio)
    log -= log.mean()
    spread = log.std()
    target = min(max(spread, MIN_CONTRAST), max_contrast)
    log *= target / max(spread, 1e-6)
    ratio = np.exp(log)
    # Each channel against the luminance, relative to the blurred colour's own balance: kept at `chroma_kept`.
    balance = np.maximum(blur(linear, HIGH_PASS), 1e-4) / local[..., None]
    chroma = (np.maximum(linear, 1e-4) / np.maximum(luminance, 1e-4)[..., None]) / balance
    chroma = 1.0 + (chroma - 1.0) * chroma_kept
    # Clamped where the stored value would leave 0.25-1.95 once squeezed (under a squeeze, only the brightest texels).
    low, high = 1.0 - 0.75 * squeeze, 1.0 + 0.95 * squeeze
    out = np.clip(ratio[..., None] * chroma, low, high)
    for _ in range(3):
        out = np.clip(out / out.reshape(-1, 3).mean(axis=0), low, high)
    return 1.0 + (out - 1.0) / squeeze, target


def height_map(displacement):
    high = displacement - blur(displacement, HIGH_PASS)
    low, top = np.percentile(high, [1, 99])
    return np.clip(0.05 + (high - low) / max(top - low, 1e-6) * 0.9, 0.0, 1.0)


def to_bytes(values):
    return np.clip(np.round(values * 255.0), 0, 255).astype(np.uint8)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--only", default="")
    args = parser.parse_args()
    only = {n for n in args.only.split(",") if n}
    os.makedirs(OUT, exist_ok=True)
    for name, polyhaven_id, side in MATERIALS:
        if only and name not in only:
            continue
        paths = fetch(polyhaven_id)
        ratio, contrast = albedo_ratio(load(paths["diff"], "RGB"), **LOOKS.get(name, {}))
        normal = load(paths["nor"], "RGB")
        height = height_map(load(paths["disp"], "L"))
        albedo = Image.fromarray(to_bytes(ratio * 0.5), "RGB")
        surface = Image.fromarray(to_bytes(np.stack([normal[..., 0], normal[..., 1], height], axis=-1)), "RGB")
        if side != SIZE:
            albedo = albedo.resize((side, side), Image.LANCZOS)
            surface = surface.resize((side, side), Image.LANCZOS)
        # The colour's chroma at half resolution (4:2:0); the surface's channels are data (4:4:4).
        albedo.save(os.path.join(OUT, f"{name}-albedo.jpg"), quality=QUALITY, subsampling=2)
        surface.save(os.path.join(OUT, f"{name}-surface.jpg"), quality=QUALITY, subsampling=0)
        sizes = [os.path.getsize(os.path.join(OUT, f"{name}-{k}.jpg")) // 1024 for k in ("albedo", "surface")]
        # The source's own mean colour (linear), for the cliffs' colours chosen by hand (`CliffRock::look`).
        mean = srgb_to_linear(load(paths["diff"], "RGB")).reshape(-1, 3).mean(axis=0)
        print(
            f"{name:10} {polyhaven_id:22} contrast {contrast:.2f}  {sizes[0]} + {sizes[1]} KB"
            f"  mean linear {mean[0]:.3f} {mean[1]:.3f} {mean[2]:.3f}"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
