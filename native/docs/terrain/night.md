# Night lights

At night the land burns its lights from the map: street lights along OpenStreetMap's paved roads, floods over its port,
industrial and railway land and its piers, red obstruction lights on what stands tall, an airfield's runway lights, the
ground lit under them, and beyond the map's pages the towns' glow to the horizon. Night only (the sun 6° up to 2° down,
as the windows, or the moon the light), the High and Ultra settings' (the features' pages carry the lamps; Medium draws
as before), presentation only. `naval_terrain::night` (`src/night.rs`), `shaders/night.wgsl` (the land's own light),
`shaders/lamps.wgsl` (the lamps); the host's side is `naval-client`'s `theatre::lamps`.

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
- **Colours**: in the Americas and elsewhere (`LampPalette::Sodium`), sodium on the major roads with some white, warm
  whites and sodium on the rest; in Japan (`White`), mercury white on the major roads, warm and cooler whites on the
  rest. Floods metal halide or sodium; jetties' low lamps as a minor road's; a runway's edges white, its thresholds
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

**Cost** (2026-10-08, 1920×1080 Ultra on the M-series GPU, shared with three other captures at the time; the renderer's
pass timestamps, p10 of 240 frames each way in alternating blocks of the lights shown and hidden, the land audit's
views): 400 m over the port of Los Angeles (27,500 lamps drawn) the transparent pass +0.06 ms and the land +0.04 ms;
from a ship 6 km off it (21,600 lamps) within the noise (under 0.03 ms a pass); 3,000 m over the basin (the glow)
+0.01 ms; 500 m straight down on Hermosa Beach (a pool on every street) the land +0.19 ms, the most. By day nothing:
no lamp drawn, and the land's night shading leaves at its first (uniform) branch; Medium draws as before (no lamps, the
night uniforms zero). Memory: the instance buffer, 4.2 MB on High and Ultra. The pools fade out from 3.5 to 5 km (a
sliver on screen beyond, the floods' own light carries a yard) and a lamp's quad is 2.5 times its core's radius, so
the overdraw stays small.

**Tests.** `naval-terrain --lib night`: a lamp packs its fields; the rules dim and put out; the night comes as the sun
sinks; every spacing divides the lattice; a runway is lit along its edges at most 60 m apart and across its ends, a
stub not at all. `naval-client --lib lamps`: a primary road across two tiles takes its lamps once each at its spacing,
all on the page; industrial land is floodlit, a pool under each flood; the palettes, rules and the street table.
`--test all -- backends::` composes `night.wgsl` and `lamps.wgsl` on every backend.

**Open.** A runway has its edge and threshold lights only: no approach lights, centre line or touchdown zone lights,
no red end lights (a threshold's bar is green from every side) and no taxiway lights. The light dome under low cloud (the towns' glow on the cloud's base) is not drawn.
The lit ground is the land's alone: the buildings' walls and the trees take no street light. A pool falls on the road's
middle line at the lattice's crossing, a few metres from a lamp staggered to the side, and at a junction the nearest
road's lamps. The glow beyond the pages is the cover's and the photograph's, not the roads'.

History (the lanes): [docs/archive/terrain-history.md](../../../docs/archive/terrain-history.md).
