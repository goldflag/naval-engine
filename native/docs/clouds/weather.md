# Weather and the cloud anchors

Where the clouds are laid, what each weather draws and how it changes, the rain, and what aircraft spotting shares
with the clouds. The march itself is [march.md](march.md); the index is [README.md](README.md).

## The cloud anchors

The weather texture is laid on the simulation's **cloud anchors** (`crates/naval-sim/src/environment/clouds`: the centres of the
216 cube-sphere cells, each cell grown by 200 km, a place reading every anchor whose grown cell holds it), so the drawn
clouds are the simulated ones wherever the camera is. `naval-clouds` builds without the simulation: it draws whatever a
`laying::Placement` lays into its `Laying` (the uniforms, the weights map, when the histories drop), and with none the
texture lies flat on the scene's grid. The game's placement is `native/crates/naval-clouds-sim` (`SimClouds`), which
keeps the laying about the camera from frame to frame; the sky asks it every frame with its own camera, so the laying
follows the camera between ticks, and `naval_clouds_sim::set_clouds` hands it each render packet's `CloudRender`:

- **The maps.** For each anchor whose grown cell meets the drawn clouds (up to four), the second-order map from the
  scene to the anchor's chart at the camera (value, Jacobian, Hessian, from `naval_sim::clouds::laying`, in f64 with
  the 100 km tile's period taken out), so the shader (`field.wgsl`, `anchor_point`) adds a few multiply-adds in small
  f32 numbers. Against the exact composition (`S` 350 km off, anchors up to 1,800 km away) it errs by 0.0094, 0.148,
  1.20 and 4.09 m at 40, 100, 200 and 300 km from the camera, inside C13's 0.01, 0.15, 1.2 and 4.1 m
  (`crates/naval-sim/tests/cloud_laying.rs`).
- **The blend.** Where more than one anchor holds a place their values are blended keeping one texture's statistics
  (`naval_cloud_field::blend`, re-exported as `naval_sim::clouds::blend`; Heitz and Neyret's histogram-preserving blending): each value through the texture's
  Gaussianizing table (built once from the tile's bilinear reads), `μ + Σ wᵢ (gᵢ − μ) / √Σ wᵢ²`, and back through the
  inverse table, the tables' round trip taken off in the weights' proportion so one anchor reads itself exactly. A
  linear cross-fade averaged the anchors toward their mean: a thinner, wider cloud over every band, and more of it to
  march. On a cell's edge and where four cells meet, the share of places past each fair-weather layer's density
  threshold is within 2 points of one anchor's and each channel's mean within 0.02 (`a_band_keeps_one_textures_coverage`).
  The shader reads the simulation's tables (`laying::WeatherTables`, a storage buffer).
- **The weights.** A 128² `Rgba16Float` map over 800 km about the camera, each texel the anchors' weights in order,
  laid again when the camera has moved 150 km; one anchor reads none.
- **The composites.** Read anchor by anchor, a sample in a band takes a fetch an anchor. With more than one anchor
  drawn, `shaders/weather.wgsl` lays the blended weather into two textures about the camera, and every pass reads
  them with one fetch: near, 2048² of the tile's own texel (195 m) 200 km each side, where the march reads the tile's
  finest level; far, 1536² of twice it, 300 km each side; three levels each, four 8-bit channels as the tile's
  (34 MB). A read off their edge, or at a coarser level than they hold, is the direct read. They are laid again
  when the drift has carried an anchor's texture half a texel (98 m) from where they have it, or the camera is 3 km
  from where they were laid: in eighths of their rows, one a frame, every eighth for the frame the laying began (the
  layer keeps that frame's uniforms); whole at a re-lay, a change of anchors or a weights map laid again. They are
  read moved by the noise anchor's drift since the frame they were laid for, the rows the latest laying has reached by
  its own, the rest by the laying before; a read across the boundary between them is the direct read. Their reads differ from the direct read by RMS
  0.016 of the weather's 0 to 1 (0.013 at the finest level near the camera, at most 0.14: bilinear between texels
  that are each a bilinear read, and eight bits; `naval-clouds-sim`'s `tests/composite.rs`).
- **The shapes.** The shape, detail and turbulence noise are laid on one anchor's chart (the one the camera read
  last while it still reads it, else its heaviest), also to second order, with a 10 km period.
- **Histories.** One `LayingUniforms` serves the march, the shadows, the ground map and the reconstruction. A re-lay
  (the scene on another chart), other anchors drawn or another noise anchor sets `Laying::relaid`, and every history
  drops. A re-lay moves no cloud (`naval-clouds-sim`'s `tests/laying.rs`: no point within 200 km of the camera by more than 2.6 m or
  55 µrad seen from it, a tenth of a pixel) and a re-centre keeps the histories.

**Cost.** `cargo run -p naval-clouds-sim --example timings --release -- --tiers high --place lat,lon --ab` measures the
cloud passes laid on the anchors against the flat laying (one anchor's path, master's) in alternating blocks under one
load, the field's clock running (Apple M5 Pro, High, noon, air, near and above summed, 2026-10-16):

| Place | Anchors | Direct read, linear blend | Composites, histogram-preserving blend |
| --- | --- | --- | --- |
| Denmark Strait, Force H (62.9° N 24.5° W) | 3 | +12 % | +4.5 % |
| An edge (0° 7.5° E) | 2 | +27 % | +4.8 % |
| Four cells meet (0° 0°) | 4 | +60 % | +4.4 % |

(Medians of two runs each, 2026-10-16; the tenth percentiles +4.4 to +5.3 %. Most of what is left is the march over
other clouds than the flat laying's at the same place: with a single anchor's texture in the composite it was +2 to
+6 %.)

`--bakes <rate>` counts the composites' slices over a minute of the field's clock at `rate`: a whole laying costs
1.25 ms where four cells meet, a slice 0.09 to 0.23 ms; spread over the minute, 0.0002 ms a frame at 1×, at most
0.010 ms at 8× and 0.030 ms at 64× (Force H, where three anchors' drifts differ most).

## Weather

What the weather does to the clouds, the sky's light and the sun (2026-10-03): `src/weathers.rs` (the layers),
`CloudLayer::apply` and `blend`, `GameSky::blend`, `shaders/field.wgsl` (`column_above`) and `march.wgsl`
(`deck_skylight`, the fine steps), and naval-client's `weather.rs` (the drift, the battle's clock).

### The layers each weather draws

Every weather fills takram's four slots, so two weathers blend parameter by parameter. With no genus named the cover
decides: takram's layers up to 0.66 (clear, partly cloudy, the maps' own skies), turning into a deck from 0.66 to 0.8 and
a storm from 0.87 to 0.94, continuously.

| Slot | Fair (takram) | Deck (overcast) | Storm | Fog |
| --- | --- | --- | --- | --- |
| 0 | cumulus, 750–1400 m | stratocumulus from 800 m, 700 m deep | nimbostratus, 250–2250 m, solid from about 600 m | stratus, 600–950 m, broken |
| 1 | cumulus, 1–2.2 km | altostratus, 3.5–4.1 km | cumulus heads, 1.25–5.75 km | none |
| 2 | high layer, 7.5–8 km | as fair | a thicker sheet at 8.5 km | as fair |
| 3 | empty | empty (a drizzle draws no shafts) | rain shafts under the cells (Medium up since 2026-10-20) | fog banks on the sea |

- **Decks** are slabs (`Layer::slab`): the broad weather channel (B, streaky) under the finer one (A, which takram leaves
  unused) for rolls and cells, a layer coverage of 0.55 with a narrow filter (0.35) so the deck is not saturated (takram's
  layers at the overcast's 0.82 have no gap and no thin place: over the weather texture, takram's low layer at 0.82 is
  saturated over 27 % of the sky at its widest height and nowhere clear, where the deck at 0.55 is saturated over 10 %,
  clear over 3 % and in between everywhere else), half a cumulus's density, and a base that undulates with the cells.
- **The storm's** deck is twice as dense (an optical depth in the hundreds under its deepest cells) and widest at half
  height, so the cells' bottoms hang ragged from the solid base about 600 m down to 250 m.
- **Fog banks** start under the sea: takram's shape-altering function thins every layer to nothing at its bottom, so
  the layer runs from −50 m with its bias set to peak at the surface, thinning to its top at 250 m. The broad channel
  makes banks kilometres across, the finer one breaks their edges; about half the sea lies in them, a few hundred metres
  of visibility in the thick of one. Rain shafts start under the sea too and rise to the lowest cloud's base, from the
  deck's own channels without shape noise so they fall under its thickest cells, a fifth of the sea under them in a
  downpour, their density with the square of the rain; none for a drizzle (0.3 or less: the overcast's 0.15). They
  were drawn from High up until 2026-10-20 and from Medium up since, so the High preset shows a downpour at range
  (`CloudTier::rain_shafts`: about 0.6 ms on Medium while it pours; "Rain as a camera records it").
- **A genus** the scene names (the battle rail, the console) draws its own layer at the base and depth the scene asks
  (`CloudScene::base_chosen`, `depth_chosen`: a genus brings its own, the rail's base or the console's depth set them);
  a base chosen without a genus moves takram's fair layers there. **A high sheet** named replaces slot 2 (cirrus:
  takram's high layer; cirrostratus: a smooth veil; cirrocumulus: grains).

### The light under a deck

takram lights every point of a cloud with the whole clear sky's light (a gradient from half at a layer's base to all of
it at its top) and the sun through octaves of multiple scattering, which fade exponentially with the optical depth to the
sun. Under a closed deck neither holds: the sky over a point is the deck, and sunlight reaches the base by diffusion, not
by octaves (at an optical depth of 30 the octaves pass a few thousandths of the sun where diffusion passes a quarter).
So the base was one evenly lit sheet of the clear sky's light.

Under and inside slabs the sky light is the light falling on the deck's top (the sky's and the sun's) diffused down
through the cloud over the point, less the sun's beam that crosses unscattered (the octaves' share): two-stream
diffusion through a conservative slab, `1 / (1 + 3/4 (1 − g) τ)` with the droplets' g = 0.85 (`deck_skylight`). τ is the
optical depth of every layer over the point, estimated from the weather alone at two heights of each layer's span above
it, with half the weather's density (what the shape and detail noise leave on average) (`column_above`). Thin patches
and gaps pass more light, thick cells less: the deck's structure from below, and a storm's base dark where it is deep.
The weight is the largest slab weight of the layers above the point, so broken cloud keeps takram's light: fair weather
draws exactly as before, and skips the estimate.

### Steps in slabs and the band

- **Near the camera**, in a slab within 600 m (fog it stands in, a deck it flies through), a step takes at most 0.35 of
  optical depth (and at least 8 m), and the ray goes on by it: takram's 50 m steps there left every pixel a different
  few samples of a dense medium, grain the reconstruction could not settle. The ray ends after a few such steps (it is
  opaque), so the cost stays bounded.
- **Thin parts** of a slab (a deck's ragged base) take up to two steps' length, rain up to four, 0.35 of optical depth
  each.
- **The fog and rain band** (the fourth layer when it starts under the sea) is crossed for tens of kilometres by a level
  ray, and every ray toward the sea now marches it: its empty air goes by in strides of 6 % of the distance, and past
  12 km the rest of the band is skipped.
- **Deep under a closed deck** (an optical depth of 60 over the point) the sun is not marched: its octaves there are
  under a fiftieth of the light the deck diffuses down.

takram's own layers keep all of takram's steps: fair weather marches as before.

### GPU cost

`cargo run -p naval-clouds-sim --example timings --release` with two scenes added (the storm's deck and rain from the sea,
12° up; the fog's banks, 4° up), 2400 × 1350, Apple M5 Pro, 2026-10-03, two rounds each of master (`0875c604f`) and this
branch alternating, load average 10–28 (other agents rendering; single frames swing by up to a millisecond). The march
pass, milliseconds a frame:

| Tier | Scene | Master | Weather |
| --- | --- | --- | --- |
| Medium | noon (fair) | 0.98–1.00 | 1.12–1.41 (the same march: the picture is identical, the spread is load) |
| Medium | 2.6 km up (fair) | 1.45–1.67 | 1.56–1.65 |
| Medium | overcast | 0.41–0.43 | 1.70–2.17 |
| Medium | storm | 0.32–0.35 | 1.28–1.99 |
| Medium | fog | 0.35–0.50 | 2.00–2.34 |
| Ultra | noon (fair) | 1.92–1.97 | 2.37–2.49 |
| Ultra | overcast | 0.80–1.40 | 2.92–2.94 |
| Ultra | storm (with rain shafts) | 0.71–0.75 | 3.92–5.63 |
| Ultra | fog | 0.77–1.18 | 3.08–3.59 |

Fair weather costs what it did. A deck, a storm or fog bank costs about what fair weather's clouds do from the air: takram's
decks were cheap because a ray stopped at their opaque base, where the weather's let the eye into their structure, and fog
and rain are marched from the camera. That is 1–1.8 ms more than master's overcast and storm on Medium, over the lane's
0.3 ms budget for those weathers; the rain shafts, the dearest single part (about 0.5 ms), were drawn from High up only
(from Medium up since 2026-10-20).
Levers left: a lower march resolution for the band, fewer octaves in slabs, the deck's base less ragged.

### Weather changes

`GameSky::blend(from, to, t)` draws the weather `t` of the way between two scenes: the clouds blend the two weathers'
layers on every call and keep every history (the march's reconstruction, the shadow maps and the ground map follow the
clouds as they follow drifting ones), the rain and lightning follow, and the sky's light (the sun's strength, the
environment's bake) takes the blended scene in steps of 1/48 of the change. The air is `from`'s until halfway and `to`'s
after (`SkyScene::lerp`), so its tables (77 ms of GPU under load, `docs/sky.md`) are computed once, not for every step.

naval-client's `weather::change(scene, queue, seconds)` starts a drift from the look on screen (the sky, the fog every
material takes, the water's light, the meshes' ambient: `weather::Look`) to the weather the scene's selection and
overrides resolve to, eased over `seconds` of the scene's clock (`weather::DRIFT_SECONDS`, four minutes, by default);
`weather::advance` moves it every frame (not on a paused one). The sea and its wind change at once, as before (the
simulation's sea answers a wind over its own seconds). The harness's `weather` command changes a battle's weather this way
(`docs/harness.md`); the developer console still applies its overrides at once (`weather::apply`).

### The sun on a battle's clock

A battle's sun moves with its clock, as the theatre's does with the theatre's: `weather::follow_clock` puts the sun of
`BattleClock::sun_at(seconds)` on the scene once it has moved 0.25° (about a minute), re-resolving the light as the
theatre's `relight` does (the sky and its bake, the clouds' light, the fog's colour, the water's light, the meshes'
ambient, the moon with the sun). The day is `battleEnvironment`'s (`naval_conditions::sun_at_hour`: up at 6, 70° at noon, down at
18, 15° an hour round the sky); a battle at an hour starts there, and a time preset's or a map's own sun starts on the
arc through it (`naval_conditions::hour_of_sun`), keeping its azimuth's offset from the day's. A battle at 16:00 sees the sun set
two hours in. The console's hour, while set, holds the sun; the reading shows the hour.

## Rain as a camera records it

Bill's photoreal batch (the rain lane). The rain was still the browser game's: every drop a crisp white line up to 0.8
opaque (its coverage times 500, held under 0.8), at least a pixel wide, lit by the clear sky's dome (no clouds) and
glinting, its slant varied drop by drop and nudged by fades and shortenings looking down and from aloft; splashes were
rings 0.9 m across and crowns 0.26 m tall, ten times a real splash, out to 140 m; and a post pass over the finished
scene laid a second fog 1.8 km deep that lifted the distance toward the clear sky's colour. In the storm stills the
streaks crossed the frame in several directions and the rings and crowns stood on the sea 140 m off.

- **The drops** are the rain's own. A precipitation p falls at 40 p² mm/h (the storm's 0.236 is 2.2 mm/h, light rain,
  since 2026-10-20 ("The storm's rain" below; it was 0.9, 32 mm/h, a heavy shower); the overcast's 0.15 is 0.9 mm/h); its drops from 1 mm up take Marshall–Palmer's exponential sizes
  (`weather::drop_slope`: larger in heavier rain, up to 6 mm), fall at their terminal speeds (Atlas, Srivastava &
  Sekhon 1973: 4 m/s at 1 mm, 8 at 3 mm, 9.4 at 6 mm) and drift with the wind (three quarters of the clouds'). So they
  share the wind's slant, the larger drops a little steeper, and converge on its vanishing point as any parallel
  motion does through a lens. The tier draws as many as the rate's density of them asks against a downpour's (a
  drizzle a few hundred), 70 % of them in a 5 m box ahead of the eye.
- **Each is the streak one exposure paints**, a sixtieth of a second: its motion relative to the camera, as faint as
  the time it covers a pixel makes it (the drop's area spread along the streak, a Gaussian across it: a 3 mm drop 2 m
  off covers a pixel it crosses for about a hundredth of the exposure). A lens focused far blurs the nearest (6 mm of
  aperture at a 52° field, the same f-number through any magnification: a drop 1 m off spreads over 7 px of 1030); a
  drop smaller than a pixel thins and fades. A camera records thousands of drops a few metres off; the tier draws a few
  per cent of them, and each stands for the rest about it (`weather::drop_weight`: a downpour's drops from 1 mm per m³
  over the density drawn, 14 on Medium's 8000 drops, 9 on High's 12000, 6 on Ultra's, 37 on Low's), so together they
  take the rain's share of the light, more coarsely grained. Drawn one for one they were invisible, even backlit:
  about a thousandth of a pixel's light each.
- **Their light** (Garg & Nayar 2004, "Photometric model of a rain drop"): a drop shows the scene behind it, refracted
  and inverted, from about 165°, so it takes the environment's mean light about the view (`env_radiance` at roughness
  1, the clouds and the sea in it), less the 6 % two refractions lose, and the 6 % its face reflects of the sky above;
  and the sun's (or the moon's) light it refracts forward (Henyey–Greenstein, g 0.75) where the clouds' shadow lets the
  beam through at the drop. So drops are dark under a dark deck and invisible against it, show faintly against a dark
  hull or sea, and shine when a low sun stands behind them. The rain and the splashes read the clouds' shadow at group
  0 in place of the renderer's camera (which they never read) and the sky's environment at group 3; temporal AA's
  motion pass keeps the camera there and reads neither.
- **Splashes** are crowns at their real size: a 3 mm drop's stands about 2 cm for a tenth of a second (scaled with the
  rate's mean drop: the storm's are about 1 cm), a translucent sheet lit by the sky on its rim and the sun's forward
  lobe, fading out from 14 m to 24 m, held to a pixel and a half with its area's share of the light, and none for a
  camera over 16 m up (the nearest sea it sees lies past them). The rings every drop sends out are the sea's own, in
  its normal where a pixel resolves them, and so is the roughening beyond (`native/docs/ocean/foam.md`, "Weather at sea").
- **The distance** is the rain's share of the haze (`aerial.wgsl`; `haze.md`, "The rain's share"): the extinction its
  drops make at its rate (0.30 per km in the storm's rain, 1.6 in a downpour) takes up to all of the haze's part of the
  visibility, so
  a ship goes at the same distance as before, but the rain's air is even from the sea to the clouds' base rather than a
  250 m surface layer, so the base and the horizon go grey into it. The post-pass veil, a second fog over the finished
  scene, is gone (`post.rs`, `haze.wgsl`, `post.wgsl`, historical).
- **Rain shafts** under the storm's cells are drawn from Medium clouds up (the High preset's), where they were from High
  up: the downpour closes in at range as the clouds' rain band, a ship 400 m off in a shaft grey with it.

**Measured** (Apple M5 Pro, 1728 × 1030, the High preset, GPU timestamps, p50 of a capture's last 240 frames; the
storm at noon from 300 m and 400 m; master's cached client and this branch's dev-client alternating, two rounds,
load average 11 to 50, `.build/native/look-rain/cost.sh`): the transparent pass, which draws the rain and the
splashes, 0.46 → 0.42–0.44 ms (master's opaque streaks covered more pixels); the aerial perspective unchanged within
the noise (the rain's share costs nothing); the veil's full-screen pass and the backdrop colour copy it read are
gone. The rain shafts on Medium: the cloud march 0.50 → 0.85–0.91 ms and the reconstruction 0.30 → 0.51–0.53, about
0.6 ms while it pours (precipitation over 0.3), nothing otherwise.

**Stills**: `.build/native/look-rain/` (`shots.sh before|after`): the lead's dusk storm set (`storm-*`), a daylight
storm at chase distance, on board and low beside her (`day-*`), the overcast's drizzle including 900 m up
(`overcast-*`), and the shafts on High and Medium clouds (`shafts/`).

**Open**:

- **The shafts' cost on the High preset** (about 0.6 ms while it pours) and their gameplay: a ship in a shaft goes from
  sight sooner than the lookouts lose her (as on High and Ultra since 2026-10-03). Medium's `rain_shafts` in
  `quality.rs` turns them back off.
- **Lightning in the rain-filled air**: the veil lit the air toward a strike; the haze takes lightning only as the
  environment holds it.
- **The near rain is coarse grained**: a few per cent of the drops a camera records, each standing for the rest. More
  drops (a denser nearest box) would make it finer at the same light; the grain is what the eye reads as streaks.

### The storm's rain

Bill's call on the rain lane's open problem: the storm keeps its 8 km (its fog end in
`assets/maps/battle-conditions.v1.json`, which the simulation's lookouts and the gunnery see to) and its rain becomes
the rate that visibility allows. The relation is the game's own (`naval_sky::haze::SIGHTING_DEPTH`, A's calibration:
the air at sea stops an optical depth of 2.42 over the visibility, where a ship's darker parts fall to 4 % contrast;
Koschmieder's 2 % is the meteorological visibility, 1.62 times it), and the rain's extinction is its drops' geometric
cross-section over Marshall–Palmer's sizes (Marshall & Palmer 1948; `weather::rain_extinction`, the rain lane's model,
in which the rain fills the haze up to the clouds' base). 2.42 / 8 km is 0.30 per km, which rain gives at 2.2 mm/h
(the extinction rises as R^0.63): precipitation 0.236 (`naval_conditions::STORM_RAIN`). That is light rain; the 32 mm/h it had would leave 1.5 km.

- **What changes**: where the camera is it rains 2.2 mm/h. The drops drawn follow the rate (72 drops of 1 mm and over
  per m³ against 562: an eighth as many streaks, and smaller), the splashes and the sea's rings with them, the rain's sound
  (`naval-audio`'s mix, 0.21 √p) drops about 6 dB, and the hulls still soak (wet from a quarter of the scale) but no
  longer run with water or pool (from 0.2 to 0.9). The haze does not change: at both rates the rain takes all of the
  storm's haze share (0.088 per km).
- **The shafts keep the downpour.** A storm's cells pour whatever falls between them: `naval_cloud_field::weathers`
  draws the shafts at the cells' 0.9 (`STORM_CELL_RAIN`, 32 mm/h) under a storm's cover (from 0.87 to 0.94) whenever
  it rains, so the storm draws the same shafts on Medium and up. A genus the scene names keeps its own rain in them.
- **The simulation** does not read the rain: its spotting clears the clouds' rain slot (`CloudField::resolve`,
  `Battle::set_cloud_weather`) and the visibility, which it does read, is unchanged. Nothing changes in the battle.
- Custom skies keep their rule: full cover in a gale still rains up to 0.9 (`battlePrecipitation`), and a chosen
  visibility is the player's.

## Aircraft spotting and shared cloud weather

The renderer-free layer types, weather profiles, local weather PNG and anchors' blend live in `crates/naval-cloud-field`
(portable, built on `naval-sky-types`), which `naval_sim::clouds` re-exports at its old paths and `naval-clouds` reads
without the simulation. `naval-sky` re-exports the genus types; `naval-clouds` re-exports the layers and adapts `SkyScene` to their common
weather profiles. Aircraft-to-ship sight rays integrate the shared map's coarse density with a mean shape fill of 0.5,
using a sight threshold of optical depth 3.912 (2% contrast). This is gameplay tuning, not a historical calibration. The lookouts march each pair's line once a second and hold the verdict between ([reconnaissance reports](../../../docs/air-operations.md#reconnaissance-reports-missions)).
The GPU continues drawing its fine shape/detail noise, so cloud fringes are approximate in spotting. The native-only
`RenderPacket.clouds` gives visual battles their authoritative layers and drift, including theatre coordinates and
clock after a handoff or re-form. Cloud layer 3's rain and sea-level fog stay the renderer's; simulation visibility
already limits sight in those conditions. Graphics quality, disabled map-view clouds and camera position never
change spotting. See [air operations](../../../docs/air-operations.md#reconnaissance-reports-missions).

`naval-client`'s `theatre_clouds` regression compares the drawn wind offset with the simulation's cloud field at
1x, 8x and 64x requested speed, then checks pause and resume while the camera moves.
