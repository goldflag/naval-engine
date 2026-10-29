# Water effects (`native/crates/naval-water-effects`)

The effects above the water, driven by simulation state: shell splashes (columns of water marched through on the
GPU), the bow's spray and the water a bow ships, the blast sheet under heavy guns, the water off a lost hull, and the
few sprites left (a crash's drops, a whale's blow). The wakes, bow waves, torpedo tracks, contact foam and the ring
waves of what strikes the sea are shaded by the ocean surface and live in `naval-ocean`: [ocean/wakes.md](ocean/wakes.md),
[ocean/bow-waves.md](ocean/bow-waves.md). The crate passes `cargo check --target wasm32-unknown-unknown`; outputs
go to `.build/native/water-effects/`. History: [native-water-effects-history.md](../../docs/archive/native-water-effects-history.md).

## The layer and its sprites

`src/{splash,particles,bow_spray,textures,lighting,layer}.rs` and `shaders/{spray,sprite}.wgsl` began as ports of the
web game's files (all historical): the water events of `CombatEffects.emit`, `BowSpray.ts` (historical),
`EffectParticles.ts` (historical, the pool), `SprayMaterial.ts` (historical: sprayRadiance and the sprite materials),
`EffectLighting.ts` (historical) and the maps (`effectTexture('droplet')`, `smokeBillowTexture`). The splashes themselves are native since 2026-10-16 ("Splash columns"): the web's
`CombatEffects.splash`, `splashBillows` and `WaterPlumes.ts` (historical) are no longer ported, and a crashing aircraft's fan of drops
is the only splash water left in the particle pools. The bows' sheets and bursts are native too since 2026-10-20 ("The
crest's spray as water", "Bow spray as water"): `BowSprayStrands.ts` (historical), its sheet material and
`sprayStreakTexture` went.

- CPU simulation, GPU drawing, as the web: `naval_effects::particles::ParticlePool` of kind `PoolKind::Water` (claim
  ring giving up the particle nearest its end, analytic drag and gravity, waterline kill, back-to-front stable sort,
  billboard/velocity/streak/water alignment) publishes 64-byte `naval_effects::instances::WaterSprite`s
  (`publish_water_into`), the pool the combat effects draw from too (`combat-effects/renderer.md`, "The effects kit").
- `WaterEffects` is one `SceneLayer` in `Phase::Transparent`, depth tested not written, straight-alpha blending (three's
  NormalBlending), linear HDR, pipelines through `RenderContext::add_scene_pipeline` (hot reload): `sprite.wgsl`
  (`fragment_spray` for billows, `fragment_basic` for drops), `column.wgsl` for the splashes' columns and the bows'
  sheets and bursts ("Splash columns"), and the shared `spray_radiance` module in `spray.wgsl`. Group 1: the lighting
  uniform, a map and its sampler (the columns': the lighting, their noise volume, its sampler and the columns). Group 2:
  the sky's environment (`WaterEffects::set_environment`, naval-sky's `naval_sky::environment` module; a black placeholder
  until the host binds the sky's).
- Draw order: the blast sheets, the splash columns and the bows' sheets and bursts (farthest first); then a crash's
  drops and the billows (a whale's blow).
- Randomness: each water event draws from `naval_effects::random::Lcg::for_event(sequence, shell id)`; the bow spray
  from one `Lcg` seeded 1 and its own integer `hash`/`noise`, in the web's call order.
- Lighting: `EffectLighting::from_sky(direction, colour, intensity, night, ambient, in_port)` does what
  `VisualEnvironment.syncLighting` does; the stills take the page's own values. Once the sky's environment is bound,
  the shade's sky light is the environment's from above (`sky_fill`, as the combat effects', so spray dims with dusk,
  night and overcast; `flash` lifts it with lightning), and unlit drops and mist take `scene_light` (the sky's and the
  sun's or moon's light against a clear noon's) instead of the authored `daylight`.
- Bow shapes come from a `WaterlineProfile` (the proxy's until the ships lane slices real models); the stem crest is
  naval-ocean's `bow_waves::stem_crest`.
- Pinned to the web: texels of the maps, the random stream, `bowShape`/`bowSide` on the proxy profile, pool sizes.
- **Spray, not cotton** (2026-10-04; `sprite.wgsl` `fragment_spray`): a storm's bow spray drew as white cotton balls,
  each billow the map's one rounded cauliflower. Each sprite now reads the billow map through a warp of its own (0.3 of
  the quad, windowed off its edges), its density raised to 1.7 and torn by value noise of its own at three scales
  (`WaterSprite::seed`, fixed for the particle's life), thinning further with age (`WaterSprite::aged`), clumped
  inside (0.45 in the gaps), with 60 % of the map's relief and 80 % of the sky's light in its shade. Every sprite
  (billows, drops, mist) fades over 0.3 of its width (at least 0.2 m) where it meets the sea or a hull behind it.
- **Spindrift** (2026-10-03), the spray a gale tears off the breaking crests, is the ocean's, not this crate's: it comes
  from the wave field's own breaking (`naval-ocean`'s `src/spindrift.rs`; `native/docs/ocean/foam.md`, "Weather at sea"). Its wisps are
  lit as this crate's white water, with `spray.wgsl`'s constants copied into `spindrift.wgsl` (this crate depends on
  naval-ocean): change them together. Since 2026-10-04 they also take the ships' and columns' shadows on the sea
  (`native/docs/ocean/foam.md`, "Weather at sea").
- **Sun shadows**: every sprite reads the sun's shadow maps at its corners in the vertex stage (`spray.wgsl`
  `particle_sun`: the near, wide and view maps), lifted up the beam by half its width, and scales the sun's or moon's
  beam by it; sprites cast none. The columns' shadows, cast and received, are in "Splash columns". The jets' and
  billows' casters they replaced: [the history](../../docs/archive/native-water-effects-history.md).

## Splash columns

The owner called the splashes "the ugliest effect in the game" and asked for them rebuilt from the ground up. Filmed
before (`effects-trial.ts`, master 5966b8d40), a 38 cm splash was a squat white bush of camera-facing blades at 0.3 s,
a fan of flat feathers at 2.5 s and a pale translucent dome from 5 s: 38 jets, 40 gas-atlas billows, 144 drops and 18
mist sprites, all flat sprites or strips turned to the eye. The photographs of real falls of shot (NH 58577, NH 82515,
NH 94003, NH 104529 of the Naval History and Heritage Command) show instead a tall, slender column, white and dense at
its foot, feathery and streaked up its sides, its head billowing wider than its neck, a bright low skirt of spray at
the water, and, after a few seconds, the water raining back out of it while the mist it shed hangs and drifts.

**A column of water, not sprites.** A splash is now one box about its water (`src/columns.rs`, `SplashColumns`), and
`shaders/column.wgsl` follows each view ray through the water's density inside it, as the clouds are drawn. The
density is a field of four bodies (`shaders/column_field.wgsl`), each where ballistics puts it:

- **The column.** Its water leaves the cavity together, slowed by a drag in proportion to its speed (k 0.7/s), so at
  time t a drop launched at v is at v·T − (0, fall, 0) with T = (1 − e^(−kt))/k and fall = (g/k)(t − T) (`flight`). One
  point of the column is then one launch speed, found back from its height: the share of the head's speed (70 m/s) at
  height y is (y + fall)/(T·70). So the column shoots up (30 m at 0.5 s, 57 m at 1.5 s), hangs (64 m at 2.6 s) and falls
  back from the sea up, its slow water first: at 5 s the water launched under half the head's speed is back in the
  sea, and the head is down by 8.5 s. Its radius at a share is the cavity's (2.5 m) plus its outward speed times T,
  growing with the cube of the share (a flaring head) and, for the slowest quarter, a skirt about its foot. Its lumps
  and streaks are noise read in those launch coordinates (across the column in its radius, along it in launch speed),
  so they rise, stretch and fall with the water: round lumps toward the head, eroded as a cloud's rim, and streaks
  along its body that tear it into strands as it breaks up (from 1.2 to 4.5 s).
- **The veil**: the mist the column sheds, the column's shape as it stood when it stopped rising (2.4 s), swelling,
  settling at 0.6 m/s and carried off by the wind from about 1 s (`Column::drift`, integrated on the CPU), billowed and
  eroded as a cloud. It holds until 3.5 s and fades over 3.5 s more.
- **The crown**: the thin, spiky skirt of water thrown out sideways at the strike (launched 0.7 ± 0.3 rad off the
  vertical at 5–30 m/s, with its own drag, torn into fingers), gone by 1.6 s.
- **The surge**: the low, lobed roll of mist the falling column raises off the sea from 2.8 s, rolling out to about
  20 m and fading over 4.5 s.

A splash of any calibre is the 38 cm one scaled, its lengths by its size (`size_of`: the scale^0.65 the jets used, ±10 %
a splash) and its times by the root of it, so a 15 cm column is a scale model under the same gravity. A shell falling
flatter leans its column along its way (`Column::lean`). Each splash lives 16 canonical seconds.

**No two alike.** (Its lobes went with the fan, "The column as a fan" below: the varieties now set the feathers.) Shown the first columns, the owner asked: "can you make it look a bit more random?" Every splash had
been the canonical one scaled, so a salvo stood as a row of identical pills. Each now draws its own `Variety` from its
number, hashed (`Lcg::for_event` gives splashes struck one after another numbers about 0.07 apart, and the first lean's
±20 % followed them in order). It sets the splash's speed (0.74–1.08 of the canonical's: how high it stands), girth
(0.7–1.4), its head's flare (0.5–1.5), its pace (0.8–1.25: how soon its head opens and its body tears), how much veil,
crown and surge it throws (0.6–1.35 each), how far its surge rolls out (with its girth, ×0.75–1.3) and how high it
stands (0.6–1.4), and a lean of up to 0.08 m a metre astray of the shell's way. Its top stands ragged too. Lobes of a
coarse billow across the column (`lobes_at`: the volume's billows read at 0.3 a radius, a slice of its own for each
splash, full from 0.45 to 0.8) are thrown 6–32 % faster than the rest (most heads little, a few in tall spires), each
lobe a head of its own above the column's.
They are the last of it standing as it collapses. A lobe spreads as the water about it does, at the width of the column
thrown at the splash's speed, so lobes never notch the column's sides. The light is marched through the column without
its lobes (`column_body`'s `lobed`: a noise read the fewer a light step, and a thin lobe shades itself little), and the
lobes are read only inside the column's rim. Two tries came first: three round jets
with a Gaussian boost stood as smooth needles, and round flat-topped ones as stubby columns on columns. The billow's
lobes match what the photographs show: a cluster of heads at different heights, one side often standing taller.

**Light.** The sun's light reaches each step through the column toward the sun (three steps doubling from 2 m) and is
scattered many times (four octaves of Wrenninge's approximation, each further-scattered octave reaching deeper and
spread wider), with their gains normalized to redistribute the incident light rather than add energy. The two-lobe
phase (0.7 forward at g 0.78, 0.3 back) has a soft shoulder on its unresolved forward peak, so backlit heads retain
their shading through the display transform. A fold between
the column's lumps keeps 0.55 of the sun (the lumps up-sun stand in its way). The sky lights it from above, dimmed by
the water over a point but retaining half its fill, and the sea a little from below. The multiply scattered sun keeps
a small diffuse floor as optical depth grows (`SUN_FILL`, 0.18), so thick water reads soft grey. Both fills scale with
the scene's light, including dusk and night. A hull or another column up-sun shades it through the sun's
shadow maps (read once a ray, lifted 18 m up the beam past its own caster, and at least 2 m before where that
caster lies on the light's ray, `before_own_caster`). So a column seen against the sun is grey in
its middle with a bright rim, as backlit cumulus and the photographs; the thin mist glows looking toward the sun. The
aerial perspective is applied at the mean depth of what the ray scattered.

**The march.** Each ray runs from the box's near side to its far side or to the backdrop's depth (the sea or a hull)
there, whichever is nearer, so a hull standing in a column cuts it and the sea swallows its foot (softly, over 1.5 m).
Steps are 1 m through the column's dense outline, lengthening smoothly as it tears thin and with the distance outside
it, and 5 m through the mist. The first sample is deterministic, within half a fine step of the box's entry. The
original whole-mist-step jitter relied on scene TAA, which retains little history over the sea: it left a changing
speckle on the column. The march now holds its picture without temporal history (`tests/gpu.rs` changes only the
render frame counter with TAA off and requires identical pixels). A sudden
change of step drew a seam across a collapsing column, and steps of 2 m left the dense face grainy over the sea, where
temporal anti-aliasing keeps no history. The noise is the columns' own tiling 64³ volume (`shaders/column_noise.wgsl`:
Perlin–Worley billows, small Worley billows and two octaves of gradient noise, filled once on the GPU, each channel
stretched over 0–1), faded to its mean where a pixel spans more than about 2.5 m.

**Resolution and reconstruction.** The columns are marched at the backdrop's copy point into `ColumnTarget`. Large
columns keep half scene resolution, a quarter of the rays. Small distant groups get full scene resolution when their
combined projected bounding-sphere squares fit 64,000 pixels (`columns::march_divisor`); they keep it until 80,000,
so a slow camera movement does not repeatedly reallocate the target. One resolution for the sorted group preserves
the columns' back-to-front blend. These are scene pixels: the client's render scale applies before TAA upscales.

`shaders/column_composite.wgsl` copies a full-resolution texel directly. At half resolution it reconstructs with a
compact tent over three texels per axis, filtering premultiplied light and opacity together and retaining the
backdrop-depth weights at hulls, wires and the sea intersection. Empty footprints skip reconstruction. Each march ray
stops at its scene pixel's depth or the nearest of its half-resolution block's four depths, so a mast or a wire a
pixel wide in front of a column keeps its outline. Order in the phase: the blast sheets,
the columns (with the bows' bursts since 2026-10-20), a crash's drops, the bow's spray.

**The cover for the smoke** (2026-10-25). The composite draws before the combat effects and the scene's depth does not
hold the columns, so a burning ship's smoke behind a column drew over it. The march writes a second target beside
`ColumnTarget`'s colour, the view depth its water lies at times its opacity (`R16Float`, blended as the colour is, so
over the colour's alpha it is the opacity-weighted depth), and `after_copies` publishes both as the cover
(`WaterEffects::cover`, `naval_render::Cover`). With no column to march, a target still holding the last march is
cleared once (the layer asks for the backdrop copy until it is). The combat effects fade what lies behind it
(`native/docs/combat-effects/renderer.md`, "Behind the splash columns").

**Shadow.** `shaders/column_caster.wgsl` draws the column's own box (not the mist's) into every sun map but the land
maps while the column stands, marching the light's way through the column's envelope in eight steps and keeping a
screen door as dense as the water stops the sun (at most 0.7 of it), written at the depth where its water lies on
average. The mist casts nothing: the eye sees through it, and it stained the sea grey from above.

**Removed.** `plumes.rs` (`WaterPlumes`), the jets' and billows' casters (`caster.wgsl`), the gas-atlas billows
(`cloud.wgsl`, `WaterEffects::set_gas_atlas` and naval-effects' `EffectsLayer::gas_atlas`), the splash billows and
mist pools, and `jet.wgsl`'s jets (its sheets were `sheet.wgsl`). `JetVertex` and `strip_indices` lived on in
`strands.rs` for the bow's sheets until those became water too (2026-10-20, "The crest's spray as water"). The foam
patch the ocean lays under a splash (`trail.rs` impacts, [ocean/wakes.md](ocean/wakes.md), "Trails") is unchanged;
seen low across the sea it is the bright ellipse under a column. The photo mode's shutter reads the column's height,
radius and speed from `columns::column_extent`.

**Seen** with `bun native/tools/effects-trial.ts --only splash,splash-near,splash-far,splash-above` (master's build with
`--binary` for the before), and quickly, without the client, with `cargo run -p naval-water-effects --example
column_sheet --profile dev-client` (a 38 cm splash over a flat sea from six cameras, a salvo, and a straddle of six 38
cm splashes 450 m off for their variety, 0.15–13 s, `.build/native/water-effects/columns/`).

**Cost** (Apple M5 Pro, Metal, 1280 × 720, `effects-trial.ts --cost 20`, the median of 20 redraws at each still's
moment, load average 20–25; master's release baseline against this branch's development build, whose GPU work is the
same): in `splash` (a 38 cm salvo falling 90–270 m off, the battle firing on), whole frames take 0.1–1.3 ms more than
master's (5.4–6.3 ms), the half-resolution march 0.3–1.5 ms, the transparent pass (now the composite and the bow's
spray) 0.8–1.4 ms against master's 0.8–1.5 ms, and the shadow passes nothing measurable. In `splash-far` (1.2 km) frames
take −0.2–1.2 ms more (march 0.14–1.2 ms). The dearest moments are 7–9 s after a salvo, its columns collapsing and its
veils overlapping. Before the half-resolution march and the cheaper mist (marched at full resolution, every step lit
through a five-step march) the same moments cost 22 ms. At 4K the march's cost grows with the pixels. With each splash
varied (`cost-random3`, load average 5–8) the march costs what it did to within 0.1 ms until a salvo's columns collapse,
then up to 0.6 ms more at 7–9 s (2.0 ms close, 1.6 ms far): the lobes stand longest, and reading them is a noise fetch
more a step inside the column (0.25 ms of it, found by reading none). Frames then take up to 1.9 ms more than master's.

**Stability and quality (2026-10-17).** The same cameras at 1920 × 1080 and 100% render scale (High otherwise
renders at 70%): a heavy straddle 350–800 m away from 50 m up, the same at 4 km, and the near view backlit at 09:00.
With eye adaptation off and 1 μs steps, fixed-mask mean absolute RGB differences (0–255) with TAA off fell from
3.413 to 0.002 near, 0.942 to 0.001 far, and 1.671 to 0.003 backlit. With TAA on they fell from 0.963 to 0.336,
2.189 to 1.188, and 0.242 to 0.093. Paused captures are byte-identical after the clouds' 240-update settling period.
The column's frozen-input GPU regression changed 1,029 channel values before the fix and zero after it.

Cost on Apple M5 Pro / Metal, `dev-client`, six 38 cm splashes 450 m away, 1920 × 1080 scene pixels: same-process
shader A/B swaps, four AB/BA pairs per age in each of two runs, 120 warm draws per switch, batches of eight to keep the
GPU busy, 30 measured redraws per switch (240 samples per variant per age). Separate-process runs were dominated by
clock/load variation: even their unchanged display pass ranged from 0.12 to 0.58 ms. The batched runs' display control
was about 0.20–0.22 ms for both variants. Median march + composite times:

| Age | Before (ms) | After (ms) | Change in combined time |
| --- | --- | --- | --- |
| 1.5 s | 1.67 + 0.35 | 1.22 + 0.38 | -0.40 ms |
| 2.5 s | 1.51 + 0.32 | 1.01 + 0.41 | -0.41 ms |
| 5 s | 1.93 + 0.32 | 1.26 + 0.47 | -0.53 ms |

`column_sheet --only straddle --size 1920x1080 --times 1.5,2.5,5 --cost 30 --out <folder>` records per-pass
samples beside its stills. `Renderer::apply_shader_changes` allows same-process comparisons; the original-shader
captures matched the preserved baseline binary to the byte. No temporal history, shape or flight changes were needed.

**Shaking in a theatre (2026-10-17).** The owner saw the columns shake whenever the camera moved, though not always,
and far off through a narrow lens stand as rectangular slabs of vertical bands. Each ray was `naval::camera`'s
`view_ray`, the near plane's point through the inverse view-projection less the eye: a point half a metre from an eye
kilometres from the theatre's render origin (which re-centres only past 16 km), so the difference kept the f32
rounding of the eye's position. At 1920 × 1080 the rays were 2–9 px off at 8–30 km through a 50° lens, 11–42 px
through 10° and 29–100 px through 4°, blocks of pixels sharing one ray, the error changing each frame the eye moved;
the box itself, drawn through `world_to_clip`, stood still and cut the misplaced water square. Near the origin the
error is under half a pixel, hence "not always". `view_ray` now turns the near plane's point in the camera's own frame
(`inverse_projection`) onto the world's by the inverse view's rotation alone: under 0.001 px everywhere. The smoke's
volumes, the blast shocks and the effects' cloud visibility take the same function. `naval-render`'s
`tests/camera.rs` reads the rays back from the GPU for an eye 14 km out. The aerial perspective's froxel rays
(`naval-clouds` `aerial.wgsl`) are built the old way; the haze varies over degrees, not tenths of one.

**Open.** Dyed splashes (the US and Japanese dye loads) need only a tint per splash (`Column::tint`, white today) and the
firing ship's dye from the simulation. The columns are not lit by the flashes or fires (the blast sheets are). A column
seen from inside its box (a splash falling on the camera) is marched from the near plane over the whole screen. The
late mist reads light grey from directly above, as the old veil did.

### The column as a fan

The owner's North Sea film showed the columns as opaque white spires whose tops read as white flames, cotton wool
close up and smooth pillars far off. The photographs of falls of shot (NARA 80-G-288886, the White Plains straddled off
Samar; the Gambier Bay under fire) show instead a fan of streaky jets out of the strike, dense and white only at its
foot, translucent higher up, its tops feathered. A Houdini trial of the same (`native/tools/effects-splash/`, its README:
a ballistic droplet-parcel simulation rendered with Karma, not a FLIP solve) found that look in the water's structure (a
few feathers of streaks with sky between them, its mass thinning upward, the water falling back from the sea up) and
in near-lossless scattering, not in any solver. So the column (`column_field.wgsl`) was rebuilt as that structure:

- **Its water found back to its launch velocity** (`thrown_at`): launched together from the cavity's rim (`SPLASH_ROOT`
  3 m × girth, 1.5 m until 2026-10-24), the water at a point left with the velocity (its offset from the rim and its height lifted by the
  fall) / T, its direction and speed. A velocity's volume is T³ that of the place it reaches, so a point's water is
  `FAN_MASS` × the fan's water per unit of launch velocity / (speed² T³) (`fan_scale`).
- **Eight feathers** (`columns::feathers`, the record's `feathers` and `tips`, zero for a bow's spray): the water leaves
  the rim as a hollow cone of sheet that tears into feathers. The tallest, from the cavity's middle, stands within 0.06
  rad of the vertical; seven ring the cone 0.12–0.5 rad off it (× the root of the splash's flare), evenly round give or
  take 0.35 of their spacing, so a line of sight crosses a few jets rather than a filled cone (which read as haze). Their
  half-widths are 0.03–0.075 rad (× the root of its girth), wider toward their tips (`TIP_SPREAD`) and as the column
  opens. Their tips' speeds are 0.72–1.0 of the splash's (the tallest 1–1.07, its `spires`), falling as they lean as
  exp(−(lean / 0.85)²) (0.62 until 2026-10-24). Along a feather its water is about level from 0.08 of its tip's speed and thins past 0.6 (by the
  seventh power, `along_share`): the sheet tearing into spray. A twentieth of the water lies between the feathers.
- **How much water**: through the axis at 2 s, as the light sees the fan, about 540 at the waterline, 100 at 20 m, 13 at
  35 m and 1.6 at 50 m. The trial's 20 m³ stopped 100, 40, a few and under 1 and read grey-white beside the photographs'
  white columns.
- **Streaks and beads in the water's directions and speeds** (4 periods a radian across, slow along: they fly, stretch
  and fall with it), finer strands within them seen close. The sheet tears into strands, more as the column breaks up and
  toward its tips, its water gathered into them (as much water in fewer strands) until it rains out; a pixel far off takes
  their mean, so a far column keeps its feathers' shape.
- **The light** is marched through the fan as the light sees it: its water spread evenly about the vertical (`fan_spread`,
  `FAN_WIDTH` 0.26 rad × the root of the flare), no feathers, as is the shadow cast (`column_caster.wgsl`). Both take the
  fan's box (`fan_box`, `columns.rs` the same): as high as its fastest water's spray, as wide as its leaning feathers reach
  (`SPLASH_REACH`). The lobes (`lobes_at`), the head's heap of water and the round column's lumps are gone; the veil is
  still shed from the round column's shape.

**The march** reads at each step the tallest feather and the ring's two either side of the point's bearing (the ring
stands evenly round from `Column.kind.z`, `columns::ring_start`), not all eight. Its steps each take at most 0.4 of the
light (`STRIDE_DEPTH`), at most three fine steps long (`STRIDE_FAN`, about a strand's width), and between feathers step on
to the nearest one's edge, three half-widths from its middle (`FEATHER_EDGE`). The strands' noise is read only where a
step stops at least 0.004 of the light (`SEEN_DEPTH`), the finer strands only close (detail over 0.6); fainter steps
take the strands' mean. The first fan, all eight feathers a step and 1 m steps through it, cost 5–7 times the round
column's.

**Cost** (`effects-trial.ts --profile release --cost 20`, 1280 × 720, Apple M5 Pro under other sessions' load, master's
release client against this): the splash columns' pass in `splash` (a 38 cm salvo 90–270 m off, the battle firing on)
takes 0.30–3.13 ms against 0.27–2.07 ms, up to 1.1 ms more, most while its columns rain out where the round ones had
fallen; `splash-near` up to 1.0 ms more; `splash-far` (1.2 km) up to 0.5 ms more, often the same. Whole frames rose up to
2–3 ms at the busiest moments (noisy under the load). In `column_sheet`'s straddle (six 38 cm splashes 450 m off), 0.69
and 1.15 ms at 2.5 and 5 s against 0.44 and 0.56 ms. The whole ring read near the vertical and the steps capped by
the feathers' widths (the review's fixes) cost about a fifth of it.

Seen with `column_sheet` (every view), `effects-trial.ts --only splash,splash-near,splash-far,splash-above` against
master's client, and the North Sea film's splash shots re-shot (`straddle`, `norfolk-hit`, `jervis-hit`, `convoy`): pillars and spires
became bushy fans of white jets with sky between them, raining out as streaks over the foam's ring; at 1.2 km bushy
tufts. Close up the strands still read a little like brushed hair, and in its first second the column's foot is a dense
rounded mass (the trial's too); no drops are drawn.

### A splash's light and shadow

The owner found the splashes neither showed nor cast shadows. Measured (`effects-trial.ts` with the column's caster
drawing nothing against as in the game): their shadow changed the frames by under 0.03 of 255 on average, under 0.03
still when it stopped all the sun. The open sea is mostly the sky it mirrors; a shadow there takes only the sun's
glitter and the light from inside the water (`surface.wgsl` `lit` and `beam`), and the column stopped at most 0.7 of the
sun. The column itself stood nearly as white on its shaded side as its sunlit one: the sky lit it as it lights a lone
drop, the whole sky about it (`SKY_GAIN` 2, as much as the sun on its side), a fifth of the sun reached its shaded side
(`SUN_FILL`), and the light marched through the fan without its gaps shaded every strand alike, its sunlit ones too.

Now, for a splash only (the bow's bursts and sheets keep their light, `column.wgsl`):

- The sky lights it as a column (`SPLASH_SKY_GAIN` 0.8): its own water hides half the sky from a point of it. The sky
  is read up and also toward its middle (its axis, leaning with it), down to `SPLASH_SKY_FLOOR` 0.35 of it.
- The sun diffused through it to its shaded side is what diffusion lets through a column of its depth, 1 / (1 + 0.75 τ
  (1 − g)), some 6 % through a hundred (`SPLASH_SUN_FILL`; a tenth since its finish, below).
- The light toward the sun and the sky crosses the fan's gaps: its depth thinned by how open the fan is there (`fan_open`,
  none at its foot where the sheet has yet to tear, up to `GAPS` 0.5 of the way where it has fanned out and torn, as it
  breaks up; 0.75, from lower down, since its finish). A single layer of clumps (the light through the open share however deep the fan) was tried first: it lit
  the core as brightly as the sunlit side.
- Its shadow stops up to 0.9 of the sun (`column_caster.wgsl` `MOST`, was 0.7): its dense foot nearly all, never quite a
  hull's.

Seen in `effects-trial.ts --only splash,splash-near` against master's client and in the North Sea film's `straddle`,
`norfolk-hit` and `convoy` re-shot at 16:36: each column shows a sunlit side and a grey shaded one, grey in its lower
body and white at its sunlit edges, where it had stood flat white. Under a low light (the trial's scene at 18.8 h, by
then the moon's) a column's shadow crosses the glitter and shades a column behind it. Not staged on a hull: hulls read
the same sun maps.

### The splash's finish

The owner's North Sea film review (item 2 of the photorealism list): close up a column read as brushed hair, the first
second's foot as a smooth white lump, and far off as cotton. Close in (`effects-trial.ts --only splash`, 270 m off and
9 m up) the lump was a smooth bell: a bright shield over the foot with a dark V either side, whatever the water's noise
did. Found by switching terms off in turn: not the veil, the crown, the surge or the shadow maps, but the sun's light
through the column (gone with `sunlit` 1, there through the full light march and the mist's shortcut alike). The light
is marched through the smooth fan (`column_envelope`), as solid where the eye sees between the feathers and strands
deep into the column; the dense foot it lit, the interior seen through the gaps it drew dark. The hair was the folds'
shading (`Water.fold`): a dark line between every pair of strands.

- **The folds** shade softly, a quarter of the way at the foot and half toward the tips (in full before). The streaks are
  coarser (`STREAKS` 3, was 4) and the fine strands seen close weigh 0.2 (0.35).
- **Clumps** (`CLUMPS`, `CLUMPS_ALONG`): the billows' noise read in launch space gathers the water into clumps, which
  fly and fall with it. Low in the column they only thicken and thin it (to `CLUMP_FOOT` 0.6 of the mean); toward the
  tips the gaps between them empty. Half of them end up to a fifth of their speed short of the fan about them
  (`RAGGED`; never past it, which the box would cut), so the tips are no clean sphere about the strike but spires. The
  march's steps keep to where the whole fan's water may lie, so a clump ending short leaves no long step over its
  neighbour's.
- **Its lower body stays whole** until it breaks up: the strands' tearing starts low (0.1, rising to 0.42 by 0.7 of its
  speed), and at its foot nearly half its water fills the cone between the feathers (`FOOT_BETWEEN` 0.45, `BETWEEN`
  0.05 above 0.7 of its speed): the eye meets the water the light lit rather than an interior it took as solid.
- **Wider and leaning**: its rim is 3 m × girth (1.5), and leaning tips slow less (`TIP_TILT` 0.85, was 0.62: a jet 0.3
  rad off the vertical reaches 0.7 of the tallest's height, was 0.4), so the fan opens in a V as the 1944 photographs'
  do. Its box reaches 0.55 of its fastest speed × the travel out from the axis (`FAN_REACH`, `SPLASH_REACH`; 0.4), as
  far as its leaning tips' spray. The crown is launched steeper (0.7 rad), half as dense and torn into fingers: from low across the sea it was a
  flat white plate at the foot.
- **Light**: through the fan's gaps from lower down and wider open (`fan_open` 0.1–0.45 of its speed, `GAPS` 0.75), a
  point in a clump deeper in it and one between clumps less (`CLUMP_SHADE` 0.7 of the difference), and a tenth of the
  sun diffused through (`SPLASH_SUN_FILL`, was 6 %).
- **Its own shadow**: the shadow maps are read at least 2 m up the beam before where its own caster lies on the
  light's ray (`before_own_caster`: the caster's march, `cast_through`, now shared), and never less than 18 m up as
  before. A grown column (its box wider for the leaning jets) is wider than 18 m, and read within its own shadow it shaded
  itself a second time. Reading past its whole box was tried first; Codex found that it lost the shadow of a hull up-sun
  standing within the box, which this keeps.

Seen with `column_sheet` (`near`, `zoom`, `side`, `backlit` against master), `effects-trial.ts --only splash`, and the
North Sea film's `straddle`, `norfolk-hit` and `convoy` re-shot against the #1876 takes. Close in the column is a
whole white body without the bell, its top torn into jets; far off the fans are wider, solid low and spread in a V; the
crown's plate is gone; the shaded and backlit sides keep their grey. A young column close in is softer than it was, and a fainter bell remains over its foot close in: the light's march
through the smooth fan (`column_envelope`) does not see the gaps the eye does.
The upsampling tent was tried at 1 half-resolution texel (1.5): a little crisper, with the 2-pixel stair back; kept.
Cost (`column_sheet --cost 30`, dev client, two rounds under a load of 20–50): the near view 0.1–0.6 ms cheaper at
2.5 s (the rays stop in its whole body), the straddle about 0.2 ms dearer at 2.5 s, either way at 5 s within the noise.

### At range

(Since 2026-10-16 the splashes are columns marched through, whose dense water lights its own shade and whose noise fades
to its mean where a pixel spans more than it resolves: "Splash columns". The measurements below are the jets'.)

Theatre gun actions are fought at 20–30 km (the fleet action realism programme), where the fall of shot is spotted
through the binoculars. A heavy splash there is a few pixels across a jet even at 16× (a 9 m jet at 20 km is about 5 px
on a 1030 px frame), and seen up-sun it was no brighter than the hazed horizon behind it: the jets' tears and gaps,
the billows' clumps and the column's shaded side averaged to the sky's own level (the brightest 0.5 % of a box about a
Hipper 20 km off, 4 s after a Bismarck salvo, 0.5 % over the sky above at 16×), and the salvo vanished. Down-sun the
same column stood out (+12 %).

`far_share` (`shaders/spray.wgsl`) reads how many pixels a jet or a billow is drawn across from its own `u`
(`fwidth`): from 16 px down to 3 px it is drawn whole. The jet's fill closes into a solid column thinning toward its
tip and its tears take the streaks' mean (so it still thins as it ages); the billow keeps its mass without its tears
and clumps; and the shaded side takes `FAR_SCATTER` (0.5) of the direct beam, the light many drops deep scatter round
to it, as a cumulus's shaded side keeps about half its sunlit side's (`spray_radiance`'s `far`). A resolved column (a
splash a few hundred metres off, or a far one at high magnification once it fills the glasses) draws as before; the
bow's sheets keep their detail at every range.

Measured with `bun native/tools/long-range.ts` (`native/docs/harness.md`, "Long-range action"; noon, a Bismarck firing on a
Hipper, the brightest 0.5 % of a box about the target 4 s after the first fall against the sky above it; stills in
`.build/native/long-range/{before,after}-*`, master 78c82552a against this branch):

| Air, bearing | Range, lens | Master | Here |
| --- | --- | --- | --- |
| Clear (35 km), up-sun | 20 km, 16× | +0.5 % | +6.0 % |
| Clear, up-sun | 20 km, 8× | +0.6 % | +4.7 % |
| Clear, up-sun | 25 km, 16× | +0.2 % | +4.3 % |
| Clear, up-sun | 25 km, 8× | +0.6 % | +2.8 % |
| Clear, down-sun | 20 km, 16× | +11.6 % | +11.6 % |
| Clear, down-sun | 25 km, 16× | +8.8 % | +8.6 % |
| North Atlantic's own (25 km), up-sun | 20 km, 16× | +1.3 % | +5.1 % |
| North Atlantic's own, up-sun | 25 km, 16× | +0.7 % | +3.3 % |
| 18 km visibility, up-sun | 20 km, 16× | +0.6 % | +2.1 % |

Down-sun nothing changed (the sunlit face was already brighter than the floor); up-sun the columns read as pale white
water where they were a blue veil. In 18 km of visibility a column 25 km off is gone either way, as the haze means
(`haze.md`: a ship goes at about the visibility, and so does her splash). Whole frames from those views took the same
time on both builds (medians 8.0–8.6 ms on master, 7.9–8.3 ms here, 1728 × 1030 offscreen, M5 Pro, load average 70–85):
the change is a few ALU instructions a fragment. Columns 1.5–6 km off at 5–20° lenses look as before
(`.build/native/long-range/mid/columns-pair.png`).

## An aerial torpedo's entry

The simulation marks both successful entries and drops that break up with `TorpedoEffect.waterEntry`, keeping the
entry velocity for the small directed splash (`WaterEventKind::TorpedoEntry`) and foam ring. The column is scaled
to 0.025 of a heavy shell's splash. Ship-tube launches keep their existing presentation.
Its track is the ocean's ([ocean/wakes.md](ocean/wakes.md), "Torpedo tracks").

## The blast on the sea under the guns

A heavy turret's blast laid a dark disc on the sea around its smoke, wider than the smoke and from the instant it
fired: the smoke's shadow (`native/docs/combat-effects/renderer.md`, "Sun shadows"), its balls drawn at the full-grown puff's size while
the flipbook draws a young blast's gas at about half of it, and a burning fireball casting as smoke. That is fixed in
naval-effects (the balls are the gas as drawn, and a fireball casts once it cools). What the sea under the guns of a
battleship really shows is a sheet of white spray: the photographs of an Iowa's broadside (Vieques, 1984) show a flat
white disc reaching 50–60 m out from her side under each turret, a sharp bright rim, a thinner middle the sea shows
through, lit orange by the fireball over it.

**The sheet** (`src/blast_sheet.rs`, `shaders/blast.wgsl`). The blast's wind tears a layer of fine spray off the
waves; the water barely moves (the pressure pulse lasts hundredths of a second and presses the sea down by millimetres),
so nothing is laid on the wave field. Where the sheet reaches is where the blast is still strong enough at the sea: its
overpressure falling as r^−1.6 and leaning along the bore as (½ + ½ cos φ)^1.3 (at right angles 40 % of the overpressure
along it, behind the muzzle a tenth, never less: `FLOOR`), its reach straight ahead 95 calibres for one barrel, growing
as the cube root of the barrels firing together (Hopkinson–Cranz scaling): an Iowa's triple 16-inch turret reaches 56 m.
The sheet is the sea within that reach of the muzzles, an egg under and ahead of them, smaller for guns standing higher
or raised to a long range, and nothing for a 5-inch mount 20 m up. Its density comes in between 10 and 24 cm calibre.
The shock sweeps it out at 340 m/s (the reach in a sixth of a second), the spray coasts 22 % past the reach over half a
second, piled into a bright rim; behind the rim it clears in about 0.6 s, the rim in 1.3 s, both torn into streaks
along the blast and clumps by noise of the sheet's own, and it drifts with the rest of the spray (the lighting's wind)
until it is gone at 3.4 s.

**Drawing it.** One horizontal quad a sheet over its footprint, at the top of its mist (2 m over the water and half the
significant wave height more), grown by the run of a grazing view's column (top ÷ 0.12). Each fragment follows its view
ray down to what the backdrop's depth puts behind it (the sea, or a hull's side) and adds up the mist on the way: two
layers whose density falls with height over the sea (the middle's over 0.8 m, the rim's piled spray over 1.6 m), read
where the ray is low in it, so a sheet seen from a deck stands out as a white band and seen from overhead is a veil,
and a hull's side or a wave crest standing up through it cuts its column short. It is lit as spray (`spray_radiance`,
flat, with the sun's shadow maps and the smoke's soft occluders at the foot of its column) and by the scene's point
lights (the renderer's list at group 0, `naval::lights`, a sheet's foot taking its cell's): the turret's flash, and the
fireball's glow, which the flash light now keeps for about a second (naval-effects' `FlashRequest::glow`). It is drawn
first in the transparent phase, under every splash and under the combat smoke.

**Feeding it.** naval-effects groups a salvo's barrels into turret blasts as before and keeps each turret's
(`CombatEffects::turret_blasts`: the muzzles' centre, the bore, the calibre, the barrels and the sequence); the client
hands them to `WaterEffects::gun_blasts` each frame (`naval-client/src/scene/effects.rs`) and the sheet's water level is the
combat sea's at the muzzles' foot. `EffectCounts::blasts` counts the sheets on the sea.

Seen with `bun native/tools/effects-trial.ts --only muzzle-above,muzzle-side,muzzle-low --stills` (an Iowa's first
salvo at a still Bismarck 6 km off: from 300 m over her engaged side, from 400 m off it, forward and 80 m up, as the
photographs were taken, and from 320 m off it 12 m up, as from another ship's bridge, where the sheets are bands of
white along her waterline); `--hours 23` for the night, where the sheets take the moon's and the flashes' light.
GPU busy at 1280 × 720 (`--cost 30`, M5 Pro, two interleaved rounds against this branch's parent, load average 25–37
from other agents' builds): the transparent pass a median 0.15 ms more over the six moments the sheets lie on the sea
seen from 300 m overhead (where they fill much of the frame) and 0.09 ms from the side, against 0.03 ms in the moments
after they are gone; single moments ran from −0.03 to +0.71 ms, the noise of this load.

**Open.** One water level a sheet, the combat sea's at the muzzles' foot. The sheet leaves no foam on the sea, and a
camera inside its mist below the quad sees none of it. It does not shade what is under it.

### Torn into strands

Seen close and low (the North Sea film's view from Hipper's deck) the sheet read as a white plastic disc: one octave of
streaks (14 cells a radian round, 22 m out) over a veil that, seen along the sea, stood as one smooth white face, and a
rim that was a smooth bright band with smooth bights. The photographs of Iowas firing show strands of spray torn out
from under the guns over sea that shows between them, frothy and ragged at the front. Now (`shaders/blast.wgsl`):

- **Strands.** The streaks are three octaves (18 cells a radian and 14 m, each 2.9 times finer round and 0.42 times as
  long), cut into strands where their sum crosses 0.3–0.8. Between them lies a thin mist, a fifth of the strands'
  density (a veil of a quarter to all of it before), so even seen along the sea the sea shows through.
- **Drops.** They are beaded with clusters of drops, three octaves from cells a metre, each 2.6 times finer, which also
  fray the strands' edges and break up their light (down to three quarters of it).
- **The rim** is streaked as the sheet is, its edge wanders by 3 % of the reach with the strands' broadest octave, and
  past it the spray thins more slowly in the strands. A stronger fray, by the strands themselves, read from above as a
  starburst of white spikes.
- **Its shade.** The thick parts shade their own undersides (half the sun's light at most, by 0.6 of their vertical
  optical depth).
- **Clearing.** Behind the rim it clears in 0.5 s (0.6 before), the rim in 1.0 s (1.3).
- **Resolution.** Each octave is drawn only where its cells are about three pixels across, its mean in their place.

Seen with `effects-trial.ts --only muzzle-deck,muzzle-above,muzzle-side,muzzle-low --stills` against master's build and in
the film's low shot filmed again: from her deck a frothy, ragged sheet streaked out from the guns, the sea showing
through; from above a white ring of spray with a ragged front. The transparent pass, where the sheet is drawn, cost no
more within its noise, with ten noise reads a fragment where it had five (`--cost 20`, two rounds against master's
build under load: 0.33–0.39 ms from her deck against 0.36–0.58, 0.44–0.62 from 320 m against 0.43–0.59).

## Bow spray

### Spray over the bow and water aboard

A bow driven into a heavy sea now throws a sheet of spray up and back over her forecastle, sheds the water it ships off
her deck edges, and wets her deck (`native/docs/ships/weathering.md`, "Wet ships: rain and the sea shipped over the bow"). `bow_spray.rs`: `Aboard`, `aboard`,
`BowSpray::{aboard, aboards, drawn_sea, over_bow, shipping_green_water}`, `Bow::on_deck`, `BowSpray::curtain` (since
2026-10-20 `BowSpray::throw`'s burst over the bow), and two streams in `emit`.

**The sea against the stem.** The hulls ride the combat sea, whose long component is four times the spectrum's peak
wavelength (242 m in a 22 m/s storm, Hs 7.5 m): a destroyer heaves and pitches with it, and on it alone the water at her
stem never came within 3.5 m of her deck edge, a storm or not. What meets her bow on screen is the drawn sea: the short
waves the coupling keeps on top, the wake and the bow wave. So the host reads it back: `naval-client`'s `wet_ships.rs`
copies a box about each stem (4 % of her length aft of it, 1.5 % ahead, a quarter of her patch's half-breadth to each side)
out of the ocean's heights about the hulls (`naval_ocean::hull_heights`, the very patches the wet band reads, already
`COPY_SRC`) through a ring of three buffers, and hands the highest into `BowSpray::drawn_sea` two to four frames late. The
box is placed by the stem as the last frame drew her, the pose the heights were evaluated about (the ocean's pass runs in
the render, after the effects; a sped-up theatre moves a hull metres a frame). The rise and the sheet's and runoff's easing
run over the whole presentation interval, not the particles' capped step, so a faster clock does not steepen a plunge. A
hull out of view, or the heights off, falls back on the combat sea with the bow wave's crest on it. The copies are a few
hundred bytes a hull, outside any timed pass.

**What comes aboard** (`aboard`), each frame, for every hull whether her spray is drawn or not:

- The sea's *head* on her stem over its design waterline, and how fast it climbs (*w*, eased over 0.15 s, restarted when
  its source changes).
- A plunge throws a sheet when its spray reaches over the deck edge: the water meeting the flare leaves it in a jet at
  about twice the speed the sea climbs the stem (the spray root of a wedge entering water, Wagner 1932), so it rises
  (2w)² / 2g over the head; `jet` = head + 2w²/g − freeboard. The sheet is smooth(−1.5, 1, jet) × smooth(1, 4 m/s, w)
  × smooth(2, 8 m/s, her speed), carried aft by the apparent wind along her bow (her way through the world plus the true
  wind against her: none under 4 m/s, all over 18). Running before the sea the apparent wind is near nothing and the
  encounter slow: no sheet.
- Green water comes aboard when the sea itself stands over the deck edge at the stem (the seakeeping deck-wetness
  criterion, Ochi 1964), fully 1.5 m over.
- Spray torn off the bow's sheets and blown aboard: 0.35 × the sheets' strength × the sea's weight (significant height
  1.5–6 m) × the apparent wind's carry.

Each gives the wetting, how far aft it lands (a sheet up to 30 % of her length, green water 22 %, blown spray 12 %), how high
it climbs over the deck edge (a sheet 1.5–11.5 m: the bridge face in the worst) and the green water standing on deck.

**Calibration** (`.build/native/look-wet-ships/scripts/measure.ts`: a Fletcher ordered to 20 m/s, 70 s into the battle,
20 s sampled every fourth frame, the drawn sea read back): into the North Atlantic's 22 m/s storm head on she makes 11.4 m/s
and throws a sheet on 3.0 % of frames (one every few waves), her bins after 20 s 0.50, 0.35, 0.20 from the stem and dry aft;
45° on the bow, 5.7 %; running before it, none (jet at most −1.8 m); into a 12 m/s sea, 1.7 %; in calm water at 18 m/s her
bow wave stands 3.2 m under her deck edge and nothing comes aboard. Green water never came over her 6.8 m bow in these
seas; it is there for lower freeboards and heavier seas.

**What it throws.** For the twelve bows seen largest (since 2026-10-20 the sheet and its billows are a burst of water
marched through with the splashes' columns, and the plunge's billows bursts off her flare: "Bow spray as water"):

- *The sheet* (`curtain`): five strands across the stem head, each the path its water takes relative to the hull, up off
  the deck (1.5–10 m over it, jittered, the middle highest) and back over the forecastle as the hull runs on under it and the
  air slows it (the side sheets' drag of 0.6 a second, keeping 55 % of the hull's way as it leaves), landing on the deck
  aft of the stem. It stands as the throw does, up within 0.1 s and falling back over 0.7 s, torn more as it falls. The
  strand batch grew by five strands a bow.
- *Billows and drops over the deck*: 45 billows (a quarter of them fine mist) and 150 streaked drops a second a side at a
  full sheet, launched from the stem head across the deck at (2·g·h)^½ for 2–10 m, outboard at most 0.6 m/s, keeping 55 %
  of the hull's way, lit and hazed as every billow and drop (`spray.wgsl`).
- *Water running off the deck* (green water, and 60 % of a sheet, draining over 2.5 s): 70 streaked drops a second a side
  pouring off the deck edge along the wetted forecastle (`BowShape::deck`, nine stations over its first 30 %: each one's
  half-breadth at the highest slice and side height), outboard at 0.6–2 m/s with the hull's way, into the sea beside her.

The web port's plunge spray (`slam`) still reads the combat sea.

**Cost**: a Fletcher's bow in the storm, the transparent pass 0.31 → 0.32 ms (`native/docs/ships/weathering.md`, "Wet ships: rain and the sea shipped over the bow", for the setup);
the aboard model is a few dozen flops a hull a frame.

**Tests**: `still_water_ships_nothing_over_the_bow`, `a_head_sea_throws_sheets_over_the_bow_and_a_following_one_does_not`,
`green_water_comes_aboard_and_runs_off_the_deck_edges` (`bow_spray.rs`).

**Open**: the sheet and its billows pass through the deck and superstructure (the depth test hides what falls behind
them; nothing splashes on the deck); the readback's box takes the highest water about the stem, so a crest beside the bow
counts as one against it.

### Bow spray as water

The owner asked for the bow's splash and mist to look more realistic. Filmed on master (`bun native/tools/film/run.ts
bow-spray-storm` and `bow-spray`, 1280 × 720, master's build with `--binary`), a plunge threw what read as something
other than water:

- **Glass.** The side sheets' strands, kicked up by the plunge (0.55 of her speed upward, half as much again outboard),
  and the sheet over the bow's five strands stood over the stem as a smooth translucent arch.
- **Cotton.** At a full plunge 70 billows a second a side heaped over the forecastle, each the billow map's cauliflower,
  growing 4–7 m/s for 2–3.5 s; the sheet over the bow added 45 a second.
- **Smoke.** A quarter of those billows were mist, growing 6–11 m/s for 4–6 s at 0.3 of gravity on the wind: in a 9 m/s
  sea they stood up the forecastle as white columns to the bridge, and in a gale they hung abaft the bridge as round
  pale discs.

**A burst of water.** Each plunge now throws bursts of water, marched through on the GPU with the splashes' columns and
lit as they are (`src/bow_bursts.rs`, `BowBurst`; `shaders/column_field.wgsl`'s `burst_*`; the same march, light march
toward the sun, multiple scattering, sky and sea fill and shadow maps as "Splash columns"):

- **When.** As a plunge passes its height (`bow_spray.rs`'s `Peak`, at most once in 0.8 s a bow): her slam (her stem
  driving into the sea she rides, past 0.1) throws one off each side of her flare, and the sheet over her bow
  (`Aboard::sheet`, past 0.06) one up over her stem head. The height is the burst's strength.
- **What.** A fan of launch velocities in the plane across her side, 43° ± 17° up from outboard (over her bow, straight
  up ± 22° across her deck). Its fastest is thrown so that its steep edge climbs, against the air's drag (0.6 a second;
  `launch_for` solves the throw), as high as the old spray did: her freeboard + 2 m × (0.15 + 0.95 × strength) over the
  crest, or 1.5 + 8.5 × strength m over her deck over the bow. Its water keeps 0.3 of her way (0.55 over the bow), its
  speed fore and aft spreading 0.15 of its own and 0.12 of hers (0.35 of its own over the bow), drawn out along the
  stretch of her side that met the sea (the sheets' reach, and 0.15 s of her way either side). As in a column, a point of
  it is one launch, found back from where it is, so its strands and lumps (the columns' noise volume read in launch
  coordinates) lie along the water's flight, its top feathers into fingers, and it tears as it flies (0.2–1.3 s).
- **Along the stretch.** It is thrown from forward aft (its after end 0.3 s later), hardest about where the plunge met
  her flare, and unevenly along it (0.7–1.15 of the fan's speed). Extruded evenly, the first bursts stood as slabs with
  a straight top and square ends. The stretch's ends soften as its water spreads fore and aft, and the noise tears them.
- **Its mist.** The fan as it stood when the mist left it (0.45 s), wider, swelling by half itself a second, settling
  0.35 m/s, carried on the wind from 0.2–0.8 s, held to 0.9 s and fading over a second, all of it gone by 4 s; a thin
  cloud (at most 0.05 of optical depth a metre), its outline pushed about by billows a few metres across.
- **Left behind.** A burst stays where the sea threw it while she runs on, so it falls astern of her bow, alongside her
  forecastle. Its box lies along her side: `ColumnGpu::frame`, a turn about the vertical (the world's own for a
  splash), and `ColumnGpu::kind` tells the shaders which lanes a record holds. The bursts merge with the splashes,
  farthest first, into one march (`WaterEffects::marched`). They cast no shadow (`column_caster.wgsl` skips them).
- **Drops** still fly out of a plunge (`slam_drops`), now 5–17 cm clots smeared along their flight (they were up to
  half a metre, white balls at their apex). The sheet over the bow's drops and the water running off her deck are
  unchanged, and what comes aboard (`Aboard`) is unchanged.
- **The crest's steady spray** kept its sheets. Its heaped water stood lower in the crest's white water and fainter
  (0.28–0.44 opacity, was 0.45–0.7), and its tufts were drawn out along their flight: both had read as cotton puffs on
  the crest. Later the same day the sheets became water too, and the heap and tufts went ("The crest's spray as
  water").

**Seen** (`.build/bow-spray/`: films of master's build and this one, `film.sh`, and stills, `shots.sh`): in the storm
film the Iowa's plunge rises white up her flare, highest forward, feathers at its top, falls back alongside her
forecastle in streaky curtains and is gone in about 1.5 s; the Fletcher's sheet over the bow rises off her stem head,
rolls over the forecastle and thins into a veil the wind takes past her forward mount. In the fair-weather film the
forecastle's white columns are a veil of spray swept aft; a gale still's round disc abaft her bridge is gone. Frozen at
one storm moment from 45 m (`--seconds 127.6` of the storm film's battle, staged with `shot.ts --orders`), master's
cotton heap and its ghost beside her are a sheet of spray. Calm and moderate stills draw as master's, and every
`check.ts` scene renders master's means to the hundredth (their references are stale on both builds).

**Cost** (Apple M5 Pro, Metal, dev-client, 1920 × 1080, load average 6–11 from other sessions; that frozen storm moment,
the median of 90 redraws, four alternating rounds of master's build and this one, a view of the bow less one looking
away from it): a burst filling about a third of the frame costs 0.13–0.87 ms more than master's billows, about 0.6 ms
as a median. A burst lives about 2.5 s of a plunge every few waves and costs nothing between.

**Tests** (`bow_bursts.rs`): `a_burst_rises_to_her_deck_and_falls_back_into_the_sea`,
`as_she_runs_on_her_spray_falls_astern_of_her_bow`, `the_box_holds_the_fan_and_follows_its_mist_downwind`,
`a_burst_lies_along_her_side_in_the_world`, `a_full_list_gives_up_its_oldest_and_bursts_publish_in_view`,
`a_throw_climbs_as_high_as_it_was_thrown_for`, and `no_water_lies_outside_its_box` (the shader's field mirrored at its
noise's worst: nothing outside any face of the box through the bursts' lives, on either side and over the bow).

**Open.** The bursts cast no shadow and are not lit by the flashes or fires. The march stops at what stands in front of
it, so a sheet over the bow behind the superstructure is hidden, but one in front passes through it, as before. The
crest's steady spray was then still the web game's sheets and sprites (since: "The crest's spray as water"). A burst
seen from inside its box (the On board view
in a plunge) is marched from the near plane over the whole screen.

### The crest's spray as water

The owner asked next for the spray a bow throws in calm water: after "Bow spray as water" a plunge's spray was water,
but the steady spray off the crest at speed was still the web game's. On master (stills of a Fletcher at 33 kn and an
Iowa at 29 kn in a 4 m/s wind, `.build/bow-crest/shots.sh`):

- **Glass.** Each side's sheet was ten strands, flat ribbons along their water's flight past her with streaks flowing
  along them: close by, a smooth glassy arch over the crest.
- **Cotton.** Billows heaped on the crest (14 a second a side) and tufts flying with the sheet (120 a second): puffs.

**A sheet of water.** Each side's sheet is now one volume of water in her own frame, marched through with the
splashes' columns and the bursts and lit as they are (`src/bow_sheets.rs`, `BowSheet`; `shaders/column_field.wgsl`'s
`sheet_*`):

- **The same flight.** Its water leaves the crest as the strands' did: from the stem aft over the sheets' reach, 1.1
  times as high as the crest there, climbing at the speed that carries it to the crest's peak (`Bow::climb_at`),
  outboard at 0.42–0.51 of her speed (the strands' mean, slowest at the stem), keeping 0.15 of her way, slowed by a
  drag of 0.6 a second. The record holds that at four stations along the stretch (the crest line's breadth, the height
  its water leaves at, how far her stem's rake and her trim move where it leaves fore and aft, its climb), read in line
  between them.
- **Found back from where it is.** Its water stands steady about her, so the water now z aft of the stem left station s
  (z − s) / U seconds ago, U its speed aft past her. A cross-section is the water of every station from the stem to it,
  which lies close together over the crest: the shader draws it as the line through its youngest water (leaving the
  crest abreast of it), its middle and its oldest, the water spread about the line (0.08 m where it leaves; for each
  metre of its flight 0.06 of its launch speed every way and 0.12 of its climb up and down), as much water on it as
  the stretch has thrown that far aft over the cross-section's area.
- **Water, not glass or cotton.** Its strands are the columns' noise read in the station and the moment its water
  left, so they lie along its flight and stream aft with the water, fine across it and a few metres long. Where it
  leaves the crest it is a film (0.3 of its spray); it whitens and tears as it flies (from 0.1 s, most by 1.1 s, its rim
  first), its outline pushed out and in by fingers along its flight (0.45 of its spread), and it swells and slackens
  with the short seas she meets, each swell running aft with the water.
- **A sea met** throws it higher (the strands' kick), and now only the water thrown then: the record keeps the kick at
  the water's ages 0, 0.4, 0.8 and 1.2 s (`BowState`, a sample every 0.1 s), so a surge runs aft along the sheet. The
  strands rose all at once.
- **No mist.** A halo of mist about it (tried at 0.1–1 of its strength) stood as smoke along her side; a plunge's bursts
  shed the mist.
- **Marched** farthest first with the columns and the bursts (`WaterEffects::marched`, sorted by distance), in steps of
  0.08–0.3 m through its water and up to 3 m through the empty box, its light read once toward the sun with the sky
  left open over it, cut softly over 0.4 m where it meets the crest and the hull, shaded by her where the ray first
  meets its water. It casts no shadow. A view that draws her bow under 0.8 pixels a metre (about 1.7 km at 1080p) shows
  none of her sheets, and all of them by 1.6: each view by its own camera (`BowSheets::publish`), so a second view
  close on her (the fall of shot) still shows them.
- **Gone** (historical): the strands (`strands.rs`, `sheet.wgsl`, the streak map), the heap and the tufts. The crest's drops still
  fly. The billows' pool stays: a whale's blow is drawn with it (naval-client's wildlife).

**Seen** (`.build/bow-crest/`: stills, `shots.sh`; films of master's build and this one, `film.sh`): from abeam and
from her bow the crest's glassy arch is torn white spray, feathered along its top, falling back onto the crest astern;
from 100 m and more it draws much as master did. Seen from astern along her side, its youngest water stands at the
stem as a narrow white blade.

**Cost** (Apple M5 Pro, Metal, 1920 × 1080, load average 5–25 from other sessions; the GPU's timing of the columns' pass,
the median of 240 redraws of one view, where master marches nothing): the sheets cost 0.12 ms with her 500 m off (most
of it the march's own target), 0.23–0.36 ms from the chase camera, 0.30 ms 40 m off her bow, 0.62 ms 15 m off it and
0.19 ms for an Iowa 50 m off. The transparent pass (the march's composite and the sprites) stays within noise of
master's. Before the mist went, the light read once and the sky left open, they cost 0.32, 0.52 and 1.1 ms there. The
strands and sprites they replace cost next to nothing.

**Tests** (`bow_sheets.rs`): `a_sheet_rises_off_the_crest_and_falls_back_astern`,
`a_kick_lifts_the_water_it_threw_as_it_runs_aft`, `a_sheet_lies_along_her_side_in_the_world`,
`sheets_publish_in_view_and_not_the_hidden_hulls`, and `no_water_lies_outside_its_box` (the shader's field mirrored
at its noise's worst: nothing outside any face of the box, either side, still or kicked); `bow_spray.rs`'s
`a_destroyer_at_28_knots_throws_spray` (a sheet off each side, low, back in the sea astern).

**Open.** The sheets cast no shadow. The columns' march draws into a full-resolution target when only small columns
stand, about 0.1 ms whatever they hold. The sheet's lit as one sample toward the sun.

### The bow's spray without drops

The owner could not stand the bow's spray, the large drops worst of all (their screenshot, low beside a battleship's
bow in a sea: soft pale discs over the crest and against her side), and liked the mist a plunge's burst carries over her
bow. They asked for the bow's spray to be more like that.

- **No drops.** The bow throws no sprites now. Its drops were sprites on the droplet map, drawn out 3–6 times along their
  flight: seen end-on from beside her hull they stood as discs half a metre across. Gone are the drops torn off the
  crest's sheets (110 a second a side at full speed, 0.12–0.4 m), out of a plunge (`slam_drops`, 160 a second a side),
  over her bow (150) and the water running off her deck (70); with them `emit`, `BowSpray::drops` and its "bow drops"
  batch, the deck's runoff, and `BowSpray::density` (the effects density only scaled them). A splash's drops and a
  whale's blow keep their sprites.
- **Every sea she meets throws a burst.** When the bow meets a sea (the kick that lifts her sheets, every 1.8–4.2 s, as
  heavy as the sea is rough: 0.35–1 of `smooth(0.2, 2.5, waves)`) and she is under way, she throws a burst off each side
  of her flare as a plunge does (`bow_spray.rs`'s `side_bursts`, shared with the plunge), lighter: as high up her flare as
  a plunge half as heavy (`MET_RISE`), 0.6 of a plunge's water for its height and 0.8 of it mist (a plunge's is 0.6),
  from a met sea of 0.08 (the kick times her way's share of her spray). So in a seaway the spray comes as the plunge's
  does: white streaks up her flare feathering into mist the wind takes aft over her forecastle, gone in a few seconds. A
  plunge throwing in the same frame throws instead. The bursts' list holds 64 (was 48).
- **In calm water** she meets no sea, so her spray is the crest's sheet alone, as before, without its drops.
- **Tried first:** a steady mist shed by the crest's sheet and carried aft on the apparent wind (marched in the sheet's
  record, two lanes more). However it was drawn it stood along her side as a flat grey band of haze, as the halo had in
  "The crest's spray as water"; a steady mist reads as smoke, a burst's passing one as spray.

**Seen** (`.build/bow-mist/`: `shots.sh` and `storm.sh` with master's client and this one, `pair.sh` side by side, and
the fair-weather film's abeam takes, `film/run.ts bow-spray --only fletcher-abeam,iowa-abeam,iowa-forecastle`): a
Fletcher at 33 kn in calm water has the same sheet with no white blobs over its crest; in the 9 m/s film a sea met
throws its burst off her flare and its mist rises over the forecastle and streams aft; an Iowa in an 18 m/s sea throws
one off her stem from ahead; the storm's plunge (127.6 s) reads as master's, a sea met adding to it.

**Cost:** a burst as a plunge's (about 0.6 ms for one filling a third of 1080p), now every few seconds a bow in a sea
rather than every few waves in a heavy one; nothing in calm water, and the drops' sprites are no longer drawn.

**Tests** (`bow_spray.rs`): `a_sea_met_under_way_throws_a_burst_off_her_flare` (the sea's weight alone: bursts in
pairs, mist in them; none in still water), `a_destroyer_at_28_knots_throws_spray` (no sprites), `green_water_comes_aboard`.

## Water off a lost hull

A capsizing Bismarck rolled 150° in 12 s and came up keel up clean and dry: nothing ran off her side as it rose out of
the sea, nor off her bottom. Now (`runoff.rs`, `HullRunoff`; for the loss as a whole, `native/docs/combat-effects/ship-loss.md`, "A ship's
loss"):

- **What sheds.** Each lost hull (`EffectShip::lost`: sinking or out of the fight, the host's), the two nearest the
  camera, is watched at points over her surface at 8 stations of her waterline profile, each the middle of its eighth
  of her length: her deck edges, the turn of her bilge (0.65 of her draft down) and her keel. A point that has been under the sea for
  3 s or more and comes up clear of it (0.8 m and the combat sea's amplitude: more than a heave in a seaway lifts her
  water line) sheds what it brought up, the more the longer it was under (full after 20 s): a deck edge pours the water
  standing on her deck off over it for some seconds (6 s, green water's freeing time aboard doubled for a whole deck
  lifted out at once), her plating runs off its film within a few (2.5 s). The rule is the water's, not the motion's: a
  hull heaving or rolling in a seaway lifts her water line clear of a crest too briefly and too little to count; a
  capsizing battleship lifts a side and deck out of the sea, an end rising as she goes down by the other lifts her
  forecastle out, and a hull floating keel up has her bottom out of it.
- **How it falls.** As the bow's thrown water does, with no sprites (the bow's drops read as pale discs and are gone,
  "The bow's spray without drops"): a burst (`bow_bursts`, marched with the splashes' columns) off each draining point
  every 1.5 s while it drains a tenth or more, spread along the eighth of her side about it. A deck's water pours over
  its edge outboard and down (3 m/s, 20° below outboard, 0.8 of a burst's water at full strength, half of it mist the
  wind takes); her plating's film runs down her side (1.5 m/s, 57° below, 0.4 of the water, 0.3 mist). The bursts'
  list holds 128 (was 64) for them.
- **On her plating** (naval-ships' `HullWetting::shedding`, `ship.wgsl`'s `shed_water`; the host's `wet_ships.rs`
  passes `HullRunoff::shedding`, the most any of her points is draining): the sea she brought up streams down her
  plating for a minute or two after (60 s to 1/e, `SHED_DRAINING`; an estimate), a film over all of it and rivulets
  down her sides and bottom (world down: a keel-up hull's water runs from her keel to the sea) moving at a metre a
  second, none on a face turned to the sea. Under the film her paint is darker and glossy: a diffusing paint of albedo a
  keeps (1 − r)(1 − r̄) a / (1 − r̄ a) of it under a film of water (Lekner and Dorf, Applied Optics 27, 1988: r about
  0.02 at the water's face, r̄ about 0.6 from within), two fifths of a dark antifouling red (`generic-antifouling-red`,
  0.14, 0.05, 0.03), where the soaked band takes a quarter (`sea_soaked`, which stays when the film has run off). Her
  fouling is as it was. Graphics' effects Low draws an even film without the rivulets.

**Tests**: `runoff.rs`' `a_hull_rolling_over_pours_water_off_what_comes_out_of_the_sea` (a battleship rolled keel up
throws bursts and sheds; upright and lost in a seaway, heaving, none; not lost, none however she rolls). The probe's
`loss.runoff_drops` (bursts thrown) and `runoff_hulls` count them.

**Cost**: some 80 points a lost hull a frame (her two nearest), a world transform and a sea height each; while she
sheds, a few dozen of the bow's bursts marched at once (as many plunges' worth). The shader's film reads one more vec4
of the ship's record and, on her alone, one noise.

**Open**: a burst stands for the eighth of her side about its point; a deck's water does not run along her deck to a
freeing port first. The breaking lane's hull sections are not followed yet.

## Run it

```sh
cd native
cargo test -p naval-ocean -p naval-water-effects      # wakes, kernels, trails, columns, bursts, playback, shaders
cargo run -p naval-water-effects --example column_sheet --profile dev-client   # splash columns over a flat sea
cargo run -p naval-water-effects --example record --release [-- --seconds 240 --seed 7]
cargo run -p naval-water-effects --example battle_sequence --release [-- --seed 7]
cargo run -p naval-water-effects --example splash_sequence --release   # particles alone over a flat backdrop
cargo run -p naval-water-effects --example water_stills --release [-- --only wake-stern] [--frames 150]
```

- `column_sheet` draws a 38 cm splash from six cameras, a salvo and a straddle without the client
  (`--only`, `--size`, `--times`, `--cost N`, `--out`; "Splash columns").
- `record` runs a native battle in `naval-sim-host` (a Bismarck and a Fletcher a side, every hull a bot, 9 km apart,
  9 m/s) through the host's own packet builder and writes a 10 Hz fixture (`fixtures/battle-<seed>.json`): 240 s gives
  655 splashes and torpedoes running in 1,686 frames. `battle_sequence` plays it deterministically and renders shots
  placed from the state: `battle/escort-chase.png`, `escort-bow.png`, `splash-{0.4,1.2,3.0,7.0}s.png`,
  `torpedo-track.png`, `overview.png`.
- `water_stills` replays recordings of the web game's scenes through `ShipWake` and `WaterEffects` with proxy hulls;
  the web driver that recorded them is gone (the scenes and the comparison: [the history](../../docs/archive/native-water-effects-history.md)).
- In the game: `bun native/tools/effects-trial.ts --only <scenes> [--stills] [--cost N]` and
  `bun native/tools/film/run.ts <film>` ([harness.md](harness.md)).

## Per frame

```rust
ocean.wake.as_mut().unwrap().update(&WakeInputs { ships, dt, splashes, torpedoes, camera, sea_height }); // CPU
effects.update(&EffectsFrame { dt, camera, ships, events, sea, waves, crest, lighting });                  // CPU
renderer.render(frame, &mut [&mut hulls, &mut ocean, &mut sky, &mut effects], output)?;
// Ocean::prepare encodes the wake's GPU half: field steps, atlas repaints, the uniform upload.
```

`naval_water_effects::drive` turns a `FixtureFrame` (what a packet reduces to: hull poses and speeds, running
torpedoes, water events) into these inputs; the client builds the same from the packets
(`naval-client/src/scene/effects.rs`).

## Tests

- `naval-ocean`'s wake tests: [ocean/wakes.md](ocean/wakes.md), "Tests".
- `naval-water-effects`: fixture round trip, proxy hull meshes, bow shapes, the blast sheets (a broadside's footprint, higher or raised guns laying less and a light gun
  none, the reach along and off the bore, sheets laid, drifting and gone), the splash columns (`columns.rs`: the column
  shoots up, hangs and falls back from the sea up; a small splash is the canonical one scaled; the box follows the veil
  downwind; a full list gives up its oldest; the shutter's extent), the bows' bursts (`bow_bursts.rs`: their flight,
  falling astern, the box's frame, no water outside the box), a GPU smoke test, the columns' noise volume filled
  and spread over its range (`tests/gpu.rs`), and `tests/fixture_playback.rs`: 100 s of a native battle recorded through
  `naval-sim-host` drives the trails, tracks, bow waves, contact foam, splash columns and spray twice with identical
  results (needs `bun run sim:content`, as naval-sim-host's tests do). The pool, the spray's maps and the random
  stream are tested in `naval-effects` (`particles.rs`, `textures.rs`, `random.rs`).
