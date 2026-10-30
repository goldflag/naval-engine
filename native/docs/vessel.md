# Any glTF ship (`native/crates/naval-vessel`)

A ship from any glTF 2.0 file, drawn on the sea as the scene's other meshes are: a visual only, with no weapons, no
modules and nothing of the simulation. It is the engine's ship for another project (`engine-example --ship`); the
game's own ships stay `naval-ships`, built from the simulation's definitions. Started 2026-10-29 (branch
`goldflag/engine-vessel`).

The crate builds on `naval-render` and `naval-sky` only (with `native-unify`); it reaches no `naval-ships`,
`naval-sim`, `naval-protocol` or `*-sim` adapter (`bun run cargo -- tree -e normal -p naval-vessel`). It reads no file
itself but in `files` (native only): a host hands `VesselModel::load` the model's bytes and a resolver for any external
buffers and images.

## Where it lives

| Path | Holds |
| --- | --- |
| `src/setup.rs` | `Setup`, the setup file beside a model (`<model>.vessel.json`) |
| `src/vessel.rs` | `Vessel`, what a setup resolves to: size, waterline, waterplane, funnels, lamps; `VesselPose` |
| `src/read.rs` | The glTF read: the refusals, the scene's meshes in model space, the materials |
| `src/images.rs` | PNG and JPEG decode, the size cap and the mipmaps (in linear light; a cut-out or blended base colour weighted by its alpha, every other map a plain box filter) |
| `src/model.rs` | `VesselModel::load`: the read, the setup and the images, the vertices in ship space |
| `src/layer.rs`, `shaders/` | `VesselLayer`, the `SceneLayer` that draws them; the BSDF and its DFG table are `naval-ships`' |
| `src/lighting.rs` | `VesselLighting`: the sun or moon, the sky's fill, lightning, the fog, the lamps' candela scale |
| `src/inspect.rs`, `src/main.rs` | `naval-vessel inspect <model> [--write]` |
| `tests/` | Ships made in code (no downloaded or game model): sizes and axes, refusals, and on the GPU the sun's side, the deck's shadow and a masthead lamp by night |

## Ship space

Metres; +X starboard, +Y up from the waterline, −Z ahead; the origin on the waterline amidships (half way between
the model's bow and stern, on its centreline). It is the game's published GLBs' frame. The world is Y up with −Z north.
`VesselPose { position, heading, pitch, roll, heave }` gives a ship's transform: heading in radians clockwise from
north, pitch bow up, roll starboard up, heave up, applied as the game's hull matrix is (yaw, then pitch, then roll).

## The setup file

`naval-vessel inspect <model.glb> --write` prints what the model holds (bounds, an axes guess with its reasons,
materials, triangles, images) and writes a starter beside it; it never overwrites one. Fields (camelCase; unknown ones
are refused):

| Field | Default | Meaning |
| --- | --- | --- |
| `version` | 1 | The format's version |
| `model` | | The `.glb`, or a `.gltf` with its files beside it, relative to the setup (no absolute or `..` paths) |
| `length` or `scale` | scale 1, noted | Her length overall in metres (the model is scaled to it), or metres per model unit |
| `up`, `forward` | `+y`, `-z` | The model's axes that point up and from stern to bow (`+x` … `-z`) |
| `waterline` or `draft` | 4 % of her length, noted | The model's own coordinate on the `up` axis's letter at the waterline (model units), or her draft in metres below it to her lowest point |
| `beam` | the model's breadth | Her beam, when the model's breadth is not it |
| `funnels` | none | `{ name, at, smoke (1), radius? }`, `at` the funnel's top in ship space |
| `lights` | none | `{ name, at, kind, color?, intensity? }`, kind `masthead`, `side`, `stern` or `anchor`; side lamps take red or green from their side; 94 cd for a masthead lamp, 12 for the others |
| `textureSize` | 2048 | The largest side a texture keeps; larger ones are halved until they fit |

`Vessel` resolves these: `name`, `length`, `beam`, `draft`, `air_draft`, `bounds`, `model_to_ship`, `scale`,
`waterplane` (bow and stern ends, beam, 21 half-breadths cut from the hull's triangles at y 0, and whether any crossed),
`funnels` and `lamps` (each lamp's arc of visibility by kind). The floating, the wake and the smoke of a host read them.

## What it draws

- Meshes of triangles, strips and fans; the default scene or the first; node transforms (mirrored nodes rewound).
- Metallic-roughness materials: base colour, metal-rough, normal (tangents from screen derivatives, so a model needs
  none), occlusion and emissive maps, each on UV set 0 or 1 with `KHR_texture_transform` and read through its own
  texture's sampler (its wrapping, nearest or linear filtering, with or without mipmaps); vertex colours; alpha
  opaque, mask and blend (blended last, premultiplied); double-sided; `KHR_materials_emissive_strength`,
  `KHR_materials_unlit`, `KHR_materials_pbrSpecularGlossiness` (its diffuse as the base colour) and
  `KHR_materials_transmission` (drawn as blended glass that keeps its reflections). Missing normals become flat.
- Light: the sun or moon, the sky's environment map, the renderer's sun shadow maps (cast and received; land's shadow
  passes are skipped), the scene's point lights, lightning, the air's haze, through the HDR path and the display
  transform. Emissive maps glow by night (20 cd/m² at 1). Navigation lamps glow over their arcs at their real
  candela, held near at hand to an obstruction light's luminance on the land (naval-terrain's lamps), so through the
  twilight, before the bloom's night limit comes in, a masthead lamp is a bright point on her mast and not a glare over
  her. A host sets the renderer's eye as the client does (`engine-example`'s `World::set_eye`), or by night her lamps
  bloom over the view.
- Several ships at once, each a model and a full transform (`VesselInstance`: a turn, a move and any scale, her
  normals turned by its inverse transpose); placed less the eye in doubles, so far from the origin they hold still. A
  transform that mirrors or flattens her is refused (`VesselInstance::check`, counted in `VesselStats::refused`, not
  drawn): a pose never mirrors her, and a mirror belongs in the model's file, where loading rewinds it. Motion vectors
  for TAA, culled and cut out as the surfaces are drawn.
- Limits: 5 million vertices and 10 million triangles a model; textures at most `textureSize`, with mipmaps.

## What it refuses

A model it cannot draw is refused with the reason, never a panic: glTF 1.0; Draco
(`KHR_draco_mesh_compression`), meshopt (`EXT_meshopt_compression`), quantized meshes
(`KHR_mesh_quantization`), KTX2 (`KHR_texture_basisu`), WebP and AVIF textures (`EXT_texture_webp`,
`EXT_texture_avif`) and GPU instancing (`EXT_mesh_gpu_instancing`), each with the advice to re-export without it;
any other extension the file requires; textures with no source; a drawn material with a map on a UV set past 1;
accessors out of their buffers (checked arithmetic), of a kind glTF does not allow for what they hold (component type,
dimensions, normalisation), with a stride shorter than an element or a bad sparse part, and indices past their
vertices, all before `gltf`'s readers touch them (they index unchecked and assert); a URI-less buffer but the first in a
GLB; a node reached twice in the scene (two parents, a cycle); a model over the limits, counted before anything is read
for it. A panic left in `gltf` becomes the load's error as a backstop. Points and lines are left out with a note; morph targets, skins and animations
are not read, so she is drawn in her rest pose.

## Afloat

`naval-afloat` floats her (heave, pitch and roll from the drawn sea) and gives her wake, bow waves, bow spray and funnel
smoke: `engine-example`'s `join.rs` turns her `Vessel` into its `Hull` (the waterplane's length, breadth and middle, her
draft, her funnels' mouths and smoke) and her pose afloat into her `VesselPose`, the two crates' ship space and angles
being the same ([afloat.md](afloat.md), the example's [README](../crates/engine-example/README.md#any-gltf-ship-afloat)).

## Not yet

The sea is not cut out of her hull, so waves wash over a low deck and the sea shows inside an open or low hull. No heel
in a turn.
