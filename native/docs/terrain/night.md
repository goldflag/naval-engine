# Night lights

At night the land burns its lights from the map: street lights along OpenStreetMap's paved roads, floods over its port,
industrial and railway land and its piers, red obstruction lights on what stands tall, an airfield's runway lights, the
ground lit under them, and beyond the map's pages the towns' glow to the horizon. Night only (the sun 6° up to 2° down,
as the windows, or the moon the light), the High and Ultra settings' (the features' pages carry the lamps; Medium draws
as before), presentation only. `naval_terrain::night` (`src/night.rs`), `shaders/night.wgsl` (the land's own light),
`shaders/lamps.wgsl` (the lamps); the host's side is `naval-landscape`'s `lamps`, and the scenario's rule
`naval-client`'s `theatre::lamps`.

```rust
land.set_night_lights(NightLights { rule: LightsRule::Dimout, palette: LampPalette::Sodium });
land.set_night_origin(origin);          // with the feature grid: the street lights' lattice in the theatre's metres
land.set_beacons(&buildings.beacons()); // world x, height over the sea, world z
land.set_runway_lights(&lights);        // night::runway_lights of each InfraLayer::runways span, in the world
land.show_night_lights(false);          // the land drawn as with none (comparisons, `NAVAL_NIGHT_LIGHTS=off`)
land.lamp_counts();                     // (on the pages, beacons, drawn last frame): `theatre_state.lamps`
```

- **The lamps** (`Lamp`, 16 bytes: chart x, z, height, kind, class, brightness and a seed) are made by the host with
  each level-1 feature page (2 km, `FeaturePage::lamps`, at most 4096) and uploaded with it into its slot's share of
  one instance buffer (64 slots and the beacons: 4.2 MB), drawn a slot at a time where its page is in view, one quad
  each, added over the scene in the transparent phase and tested against its depth. The vertex stage stands a lamp on
  the ground, drops a street light off built-up land (the window's cover, four texels 60 m apart), a flood over the
  water (a harbour's land drawn over its basins) unless on a pier, and two in three inland floods (over 450 m from the
  sea), and draws it at least 1.25 pixels across facing the camera, its light spread over them (a flood's energy kept:
  a far port's thousands sum to what they are; a street light and a beacon a little brighter, as the eye picks them
  out). The air dims
  each on its way (`aerial_apply_additive`).
- **Street lights** (`night::street_light`): by the road's class, read as its tone on the pages, a motorway's every
  55 m 12 m up to a service road's every 60 m 6 m up, brighter on the major roads; none on a driveway, an alley, a
  parking aisle or a paved track. Each stands where its road crosses the lines x = k s or z = k s of the theatre's
  metres (whichever the road runs along more), so the lamps are fixed to the world, a road cut at a tile's edge is lit
  alike on either side, and each tile keeps only those in its own square (its features reach past it). Staggered
  either side of a road 8 m wide or more, on one side of a narrower.
- **The ground under them** is the land's own shading (`night_land`): the light of the two lamps either side along the
  nearest paved road, each an even lamp's cosine-cubed fall-off from its foot, times the land's albedo, so the pools
  show the asphalt, the pavement and the front gardens as they are. The land finds the lamps without reading them: the
  road's class from its tone, its direction from the distance field's gradient (two reads more, near a lit road only),
  the lamps on the lattice (`LATTICE_M`, 138.6 km, the least common multiple of the spacings, and the chart's origin
  there, `set_night_origin`). Where a pixel is coarser than a third of the spacing, the pools' mean along the road;
  coarser than the pools across, a band holding as much light (the features' filter): no shimmer at range.
- **Floods** (`theatre::lamps::FLOOD_M`): masts 24 m high on a jittered 110 m grid over port, harbour, industrial and
  railway land (in a past period only where its ground stood, `Roads::stood`), on a 50 m grid over a broad pier
  (mapped as an area), metal halide or sodium; each with its pool on the
  ground beneath, a Gaussian 52 m wide, so a terminal's pools merge into a yard lit nearly evenly. A narrow pier (a
  line: a jetty, a marina's pontoon) takes low, dim lamps every 45 m instead.
- **Obstruction lights** (`night::beacons_of`, read through `BuildingsLayer::beacons`): aviation red at the top of each
  chimney and crane over 25 m and at the roof's corners of anything 60 m or more, half of them flashing, about 37 a
  minute. Read from each chunk's mesh as it is uploaded (its vertices' kind, and each building's highest and lowest in
  25 m cells), handed to the land when the buildings change.
- **Runway lights** (`night::runway_lights`, of each runway `InfraLayer::runways` hands, as it changes): white edge
  lights 3 m out from either paved edge, evenly from threshold to threshold at most 60 m apart (the FAA's 200 ft), and a
  bar of green threshold lights 3 m apart across each end, 40 cm over the paint. They share the beacons' part of the
  buffer (first, the beacons after them) and burn as the street lights by the rule (a quarter under the dimout, out under
  the blackout but for a leak); a few hundred an airfield (LAX's four runways about 550).
- **The towns' glow** beyond the pages (16 km a side about the camera): the window's built-up cover, brighter along
  the photograph's bright streets and yards (its local contrast against its broad sample) and by district, as points
  in 46 m blocks where a pixel can hold them and their mean beyond, so a city glows to the horizon.
- **The towns' light in the sky** (2026-10-29): the light the lit land sends up, which the air over a town scatters
  toward the eye and a cloud's base takes from below. See "The towns' light in the sky" below.
- **Colours**: in the Americas and elsewhere (`LampPalette::Sodium`), sodium on the major roads with some white, warm
  whites and sodium on the rest; in Japan and Taiwan (`White`; since 2026-10-29 Taiwan's street lights are LED,
  the buildings' Taiwan with Kinmen and Matsu, `region::taiwanese`), mercury white on the major roads, warm and cooler whites on the rest. Floods metal halide or sodium; jetties' low lamps as a minor road's; a runway's edges white, its thresholds
  green.
- **The scenario's `lights`** (`lit`, the default; `dimout`; `blackout`; the editor's Conditions): the dimout is the
  Western Defense Command's of August 1942 on the American west coast (street lights hooded and at a quarter of their
  light, seen only from above; floods at a fifth; no glow over the towns; obstruction lights at half; the windows'
  lamps unshaded in 15% of the rooms and through the others' shades at 6% of their light (buildings.md, "Buildings", "Lamps in the
  windows"), the moored boats' cabins at 15% of their usual share, a lighthouse's lantern and a factory's roof glazing at 2% of
  their light); a blackout leaves only a rare leak (a lamp in 250 at a third of its light, the windows unshaded in 3% of
  the rooms and a line between the blackout cloth's halves in one in five, the cabins at 3% of their share, the
  lanterns and glazing out). The windows' share is `BuildingsLayer::set_windows` (`params.z`), the
  cabins' `InfraLayer::set_windows`.

## The towns' light in the sky

From the sea a lit town stands under a dome of its own light, brightest low over it, and under a low overcast its
cloud's base glows over it far brighter than the clear sky does (Kyba et al. 2011, "Cloud coverage acts as an amplifier
for ecological light pollution in urban ecosystems"). One source draws both, the land's own glow, so the sky never shows
more town than the land does:

- **The lit map** (`TerrainLayer::ground_glow`, `night::TownGlow`): the window's cover in 64² cells (1.5 km on a sea
  window, 3.7 km on an air window), each its share of built-up land as the land draws it (today's class built up, not a
  past period's country), with that glow's radiance at full strength (`night::GLOW_LUM` 0.04 in its palette's colour,
  `night::glow_color`, the same as `night.wgsl`'s: a test holds them alike) by the scenario's rule (`glow`: lit 1, the
  dimout 0.06, the blackout none) and the night (`night_of`). `None` without surface maps, by day, on Medium (no lamps:
  the land draws no glow there either), with `NAVAL_NIGHT_LIGHTS=off` or under the blackout: the sky then draws as
  before, bit for bit. A rewrite of the window's cover (`queue_cover_patch`, a late answer of the period's building)
  counts the cells it crosses again, from a bit a texel of today's built-up land kept with the map, and gives the map
  a new version. The host hands it to the sky each frame (`GameSky::set_ground_glow`, in `SeaScene::update_land`); a
  change of what the clouds take (a new map, the rule, the palette, the night, hidden or shown) marches a held or
  paused picture's converged clouds again (`CloudLayer::air_changed`).
- **The light in the air** (`naval_clouds::glow`, `shaders/glow_field.wgsl`): each lit cell sends its light up by
  Garstang's emission (Garstang 1986, "Model for artificial night-sky illumination": a share G(1 − F) of the lamps' light
  reflected by the ground as a Lambertian surface and a share F sent up directly, strongest toward the horizon,
  0.554 F ψ⁴, with his G 0.15 and F 0.1), as much as the land's glow sends (π × its radiance × the lit area). A compute
  pass sums every lit cell at each point of a grid of 80² columns over the window and a quarter of its side past each
  edge, at 9 heights from 31 m to 8 km (doubling), through the air between (the haze's and the atmosphere's extinction
  at the sea over their scale heights: `Haze::extinction_at_sea`), into the fluence and the net flux of that light per
  unit radiance. It runs when the map changes or the air moves by 4 %, not each frame, before the environment's bake
  that reads it (the header's write lands before every pass of the frame), into the sky's aerial buffer after the
  volume (`naval_sky::aerial::GLOW_OFFSET`, 0.9 MB more), with a header the host's light rewrites as it changes (the
  night, the rule, the land's offset).
- **The glow in the air** (`shaders/aerial.wgsl`, `town_light`): the aerial perspective's volume scatters it toward
  the eye at each of its steps, the net flux as a beam from the land by the haze's phase (Cornette–Shanks, g 0.75) and
  the atmosphere's (Rayleigh's), the rest of the fluence as from every side; over the clouds that cast the ground shadow
  only their mean shade's share. So everything the volume dims takes it (the land, the sea, the hulls, the clouds'
  front) and the dome too (its slice): from the sea the town's dome sits on the horizon over the coast, low and warm,
  brightest where the eye looks over the town toward it (the haze's forward scattering) and falling off away from it;
  inside a town the whole sky is brighter.
- **The cloud's base** (`shaders/march.wgsl`, `town_light_from_below`, and the environment's bake in `hooks.wgsl`, so
  the sea reflects it): the field's upward flux at each lit sample of the march, through the cloud the layers' weather
  puts under it as the sea's bounce is taken (`GROUND_FILL`), at the strength that leaves a thick deck's base sending
  back 0.7 of the light from below (`TOWN_BASE_ALBEDO`, as the star shells' base). Over a town the base glows in the
  lamps' colour and fades past its edge over a few kilometres (the light's spread from the base's height).

How it looks (2026-10-29, Taipei at 21:30 with a gibbous moon up, the town stills' views, `.build/native/towns/after2-*`
against `off2-*`, which hide it, and lane A's befores): under a 90 % stratus deck the sky from the streets goes from
black to a lit warm grey, brighter toward the horizon (its base about 0.6 of the town's own mean brightness from
1,000 m up, as a deck of albedo 0.7 over a town that fills the sky should be), and the deck over the town glows from
the tower and from the sea; under a clear sky the air over the town and the dome low over the coast from the sea are
brighter by about the natural sky's own, and the eye adapts a little lower for it. The clear dome is faint beside the
moon's aureole: the land's glow (`GLOW_LUM`) is set to the game's night, and the sky takes that light as Garstang's
model sends it up, with no gain of its own. `NAVAL_TOWN_GLOW=off` (or `naval_client::land::set_town_glow_shown`) hides
it for comparisons, the land's lights kept.

**Its cost** (2026-10-29, 1600 × 900 High in the town stills, Apple M-series GPU shared with other sessions, load 4-10;
the renderer's pass timestamps, medians of five half-second windows each way, `NAVAL_LOOK_GLOW_FRAMES`): the aerial
perspective +0.02-0.04 ms; the clouds' march under the overcast +0.07-0.15 ms (its lit samples read the field), under a
clear sky within 0.04 ms; the dome and the rest within the noise. Laying the field (`ground glow`, 57,600 points over
the window's lit cells, at most 4,096) runs once when the window's maps or the air change, not in a steady frame. By
day, at sea, on Medium, under the blackout or hidden: the header's `w` is 0, nothing is laid or read, and the sky is
drawn as before.

**Cost** (2026-10-08, 1920×1080 Ultra on the M-series GPU, shared with three other captures at the time; the renderer's
pass timestamps, p10 of 240 frames each way in alternating blocks of the lights shown and hidden, the land audit's
views): 400 m over the port of Los Angeles (27,500 lamps drawn) the transparent pass +0.06 ms and the land +0.04 ms;
from a ship 6 km off it (21,600 lamps) within the noise (under 0.03 ms a pass); 3,000 m over the basin (the glow)
+0.01 ms; 500 m straight down on Hermosa Beach (a pool on every street) the land +0.19 ms, the most. By day nothing:
no lamp drawn, and the land's night shading leaves at its first (uniform) branch; Medium draws as before (no lamps, the
night uniforms zero). Memory: the instance buffer, 4.2 MB on High and Ultra. The pools fade out from 3.5 to 5 km (a
sliver on screen beyond, the floods' own light carries a yard) and a lamp's quad is 2.5 times its core's radius, so
the overdraw stays small.

**Tests.** `naval-terrain --lib night`: the towns' glow's radiance is the shader's and follows the rule (the dimout
faint, the blackout none) and the palette (Japan's whiter); the lit map counts today's built-up land, and a rewrite of
the cover counts the cells it crosses again under a new version. `naval-clouds --lib glow`: only lit cells send
light; nothing burns by day or with nothing lit, and the grid reaches past the map in the world's metres; the air
lays it again only when it moves; a change of the light marches converged clouds again; the pass's parameters match
the WGSL.
`naval-terrain --lib night`: a lamp packs its fields; the rules dim and put out; the night comes as the sun
sinks; every spacing divides the lattice; a runway is lit along its edges at most 60 m apart and across its ends, a
stub not at all. `naval-client --lib lamps`: a primary road across two tiles takes its lamps once each at its spacing,
all on the page; industrial land is floodlit, a pool under each flood; the palettes, rules and the street table.
`--test all -- backends::` composes `night.wgsl` and `lamps.wgsl` on every backend.

**Open.** A runway has its edge and threshold lights only: no approach lights, centre line or touchdown zone lights,
no red end lights (a threshold's bar is green from every side) and no taxiway lights. The towns' light in the sky is
the cover's, not the lamps': a lit port or an airfield outside the built-up class sends none up, and a page's lamps
add nothing to it. Its grid is coarse (1.8 km about a sea window): a town smaller than a cell lights its own air as a
cell does. The clouds that are not the ground shadow's take none of the light out of the air over them.
The lit ground is the land's alone: the buildings' walls and the trees take no street light. A pool falls on the road's
middle line at the lattice's crossing, a few metres from a lamp staggered to the side, and at a junction the nearest
road's lamps. The glow beyond the pages is the cover's and the photograph's, not the roads'.

History (the lanes): [docs/archive/terrain-history.md](../../../docs/archive/terrain-history.md).
