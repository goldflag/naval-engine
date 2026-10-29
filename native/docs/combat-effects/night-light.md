# Combat effects: night light, smoke screens and lights at range

## Smoke screens, searchlights and star shells

The battle packet carries `smoke_screens`, `searchlights` and `illumination` from the Rust simulation, as its own
`SmokeRender`, `BeamRender` and `FlareRender` (positions in the packet's scene frame, `RenderPacket::frame`).
`naval-client/src/scene/effects.rs` interpolates smoke and flare positions; `naval-effects/src/tactical.rs` publishes
their instances without advancing a second smoke simulation. A screen uses its authoritative horizontal radius,
vertical half-height and density. `gas_sprite.wgsl` clips the ellipsoid chord at opaque scene depth and integrates
twelve samples of billowing smoke within it. The fire's existing Worley volume feathers the rim and gives its
crowns and folds different sun and sky exposure; overlapping withdrawal screens no longer draw smooth pale
capsules (2026-10-07). The pattern moves with the authoritative puff, rises slowly with its age and stays continuous
across theatre recenters. Bulk extinction remains `0.025 × density` per metre; the simulation's lookout rays still
use their uniform authoritative volumes. The scene and smoke-occlusion pass draw the same billows, including
with the camera inside a screen and in the emitting ship's binoculars. `tests/gpu.rs` checks visible folds,
repeatable redraws and concealment from inside a screen.

Searchlights light what they hit as directional cones in the shared `naval::lights` shader; illumination flares as
ordinary point lights after ignition, at the candle swinging under its canopy. Both are one light each
(`Searchlight::light`, `Flare::light` in `tactical.rs`), which the lit layers and the light in the air
([Light in the air at night](#light-in-the-air-at-night)) both take: real candela (`illumination.rs`: a star
shell's candle by the calibre of the gun that fired it, a 36-inch lamp's beam by its power) scaled with the night
(`naval_render::lights::candela_scale`), beside every other light in the renderer's list of the 64 brightest at the camera; the brightest on what the
camera sees cast shadows through maps of their own (`native/docs/lighting/lights.md`, "Lights at night"). This visual budget never limits
simulation illumination or sensor reports. Theatre reanchoring shifts volume centres, lamp origins and flare
positions while keeping directions and velocities in their original frames.

## Light in the air at night

Night fights were the weakest look left: a searchlight lit what it hit but no beam showed in the air, and a star shell
was a point light with a 6 m glow sprite. `air_light.rs` and `shaders/air_light.wgsl` draw what the air scatters
toward the eye from every beam and burning star shell, and what those lights show of themselves. True night keeps no
other light in its air: with no beam on and no flare burning nothing is drawn or uploaded.

**The model.** Single scattering along each view ray up to the scene's depth (the beam ends at what it hits):
`σ_s(x) p(θ) I / r²`, times what the air (and a beam's smoke screens) let through from the light to `x`, times what
the air, the clouds and the gas let through from `x` to the eye.

- `I` is the light the lit layers take (`Searchlight::light`, `Flare::light`), so the beam and the spot it lights are
  one light whatever their intensity; with it its reach (three's window) and a searchlight's cone and feathered edge.
- `σ_s` is the air the aerial perspective draws (`naval_sky::haze`, `native/docs/haze.md`; `AirMedium::of`, set each
  frame by the client): the weather's haze at sea (the visibility's `SIGHTING_DEPTH / V` less the atmosphere's share),
  densest at the sea with its 250 m scale height up to the layer's top, its albedo 0.95; the rain's share of it
  (`GameSky::rain_in_haze`: the drops' geometric cross-section, even up to the clouds' base); the atmosphere's share
  at sea. Smoke screens add the simulation's `0.025 × density` per metre with the billows `gas_sprite.wgsl` draws
  them with (single-scattering albedo 0.95, white screening smoke: an estimate).
- `p` is the haze's Cornette–Shanks phase (`g` 0.75), for the rain (large drops scatter as the haze does, the
  aerial perspective's assumption) and the smoke too: looking toward a lamp down its beam is some 250 times brighter
  than from abeam, and a beam seen from just behind its lamp is a long, even shaft.
- The way from the eye: the aerial volume's transmittance, the clouds the composite drew (their opacity, in the share
  of the ray's run through their heights in front), and the smoke occlusion map's gas (depths along the view's
  axis, as the map holds them; 30 m into a level's gas, about a screen's `1 / 0.025`). The way from the light: the
  air's optical depth at the height half way, and for a beam the screens' bulk chords.

**The integral** is taken in the scattering angle about the light: with `θ = π/2 + atan((t − t_c) / D)` (`D` the
light's distance from the ray, `t_c` where the ray passes it) `dt = r² / D dθ` cancels the inverse square, and samples
spaced evenly in `P(θ) = ∫ p dθ` (a table, `phase_table`, inverted exactly by search) cancel the phase. What is left
(the haze's height, the light's path, a beam's edge) varies slowly, so four samples for a beam and three for a flare
carry it.

**No rings (2026-10-20).** With the camera inside a beam, looking back toward its lamp (night-lights' overcast chase
view, inside Takao's beam), the glow about the lamp broke into concentric rings. Not the march's steps: the table. Its
64 entries stood 2.9° apart and the shader takes `P` as linear between them, so the phase its samples follow is
constant over each step and fell by up to a third from one step to the next in the haze's forward peak (`g` 0.75:
half the integral lies within 10°). Along a ray whose own angle to the lamp is ψ the glow sums that phase from ψ to the
lamp, so its brightness turns at each step's angle: Mach bands about the lamp. The table now has 256 entries at the
angles `π (k / 255)²` (`table_angle`), finest ahead of a light, where the phase holds within about 5 % of itself from
one step to the next, and a beam weighs each sample by the true phase over the table's (`held_phase`), so what it sums
is the true phase whatever the table holds; its samples move each frame and the temporal filter resolves what the
weights scatter. The star shells' halo, sampled the same each frame, keeps the table's phase, now in steps of a few
per cent. Rays through a light's near field (the mirror's radius over the half angle's tangent, 17 m for a
36-inch lamp at 1.5°; a flame's 0.25 m) take `r² + r₀²` and the phase at each sample instead.

**Beams.** The view ray's span inside the cone (analytic, the forward nappe only, out to the light's reach and the
scene's depth) is integrated at full resolution, jittered for temporal anti-aliasing. The screens the cone passes
through (`sphere_in_cone`, at most 32 a frame) light up inside it, four samples a chord, their light dimmed by the
screens nearer the lamp (each chord's ends, between them in proportion), and the beam beyond a screen is dimmed by it.
From inside the beam the lamp shows its mirror (a 36-inch lamp's, `LAMP_APERTURE_M`) at the beam's luminance toward
the eye, spread over at least a pixel with its light kept. A ship standing in a beam does not yet cut it: the lamp has
no shadow map here (see the follow-ups).

**Star shells.** The simulation moves a flare (its flight, then the wind's drift and a 4 m/s sink from a 600 m burst:
the US 155 mm M485's are about 5 m/s and 600 m, globalsecurity.org/military/systems/munitions/m485.htm); the swing is
drawn only: the candle hangs `SHROUD_M` (3 m) under the canopy's centre, a spherical pendulum of period `2π √(L / g)`,
3.5 s, about 10° (`SWING_RAD`, an estimate) on an ellipse and phase from its id (`Flare::candle`), and its light swings
with it. Its flame is a 0.25 m source of the light's candela (magnesium white, the light's colour). Its smoke stays in
the air where the candle left it, so the trail is the candle's path through the air as far as it has sunk (at most
200 m; `Flare::trail`, a point every 2 s of burning, each kept with the same smoke as it ages, and a straight piece of
the column between each two). The canopy glides off its sink at 0.8 m/s and turns a circle in 10–18 s (`GLIDE_M_S`, by
its id), which leaves a corkscrew some metres across, and the candle and its light move with it; the air's eddies
(0.25 m/s lasting 15 s, `Flare::smoke`) carry older smoke further off that line, some metres at the top. The column
spreads by turbulent diffusion (0.5 m²/s); billows rolled from the fire's Worley volume where the smoke holds still in
the air swell, draw in and thicken it, none at the candle and fully 40 m up; and every burning flare lights it, `I / r²`
by the smoke's phase, where each piece passes nearest the ray (`flare_trail`: 8 g/s of oxide smoke at 3 m²/g, albedo
0.9, `g` 0.6; all estimates). Until 2026-10-25 the trail was a straight, even column over the candle, lit by its own
flame only. Its glow in the haze and rain (the same integral as the beams') is drawn at half resolution at the
backdrop's copy point (`AirLight::draw_halo`, a pass of its own, `air light halo` in the GPU timings), sampled the
same each frame (the glow has no edge to jitter over), and upsampled by the composite weighing each of the four
nearest texels by how near its depth is to the pixel's, so the glow behind a hull does not bleed over her edge.

**The cloud base.** A flare under the clouds lights their base: the base sends back 0.7 of the light from below (a thick
stratiform deck's albedo, 0.6–0.8), `cos / r²` of the flare's, where the clouds cover the pixel, taken at two heights
through the base's lowest 150 m (`BASE_RAMP`: a base is ragged and the light goes some way in), so a far patch seen
nearly edge-on is a soft band and not a line. It is a lit patch over the clouds the composite drew, not a term in their
march; seen from above the deck it draws nothing.

**What it looks like at master's intensities.** The light list's values (a searchlight `range² × power × 1.5`, 1.2 ×
10⁸; a flare `range² × 0.5`, 4.5 × 10⁶, which lights the sea under it with twice the noon sun's irradiance) are the
night-lights lane's to set. Against real values (a 36-inch lamp about 1.5 × 10⁸ cd, a 5-inch star shell about 5 × 10⁵)
the flare stands ten to thirty times too bright beside the lamp, and its air goes white within a kilometre or two;
`.build/native/look-night-air/` holds a preview with the flare a tenth as bright. The air takes whatever they set.

**Tools.** `shot.ts --camera effect:beam[:n[:m]]` (from `m` metres abeam of the nth beam's middle),
`effect:lamp[:n]` (down it from beside its lamp), `effect:flare[:n[:m]]` (the nth burning star shell from below); each
prints the world camera it resolved to, for a client without these readings (a baseline). `--air-ab <rounds>` times
the frame and the passes with the light in the air drawn and not (`layers {air_light}`); `NAVAL_AIR_LIGHT=off` starts
a client without it. The RPC's `battle_state` lists `searchlights` and `illumination`.

**Cost.** Apple M5 Pro, dev-client, High, 1728 × 1030, overcast night, 2026-10-20, load average 6–11, the composite and
the halo's passes against the same frame without them (`--air-ab 3`, 90 frames each): a staged busy fight (eight star
shells burning, five beams, a smoke screen in one) 0.53 ms from the chase camera, 0.77 under a flare, 1.04 abeam of a
beam through the screen (its smoke's chords the dearest), 0.43 from 450 m up; of it the halo 0.33–0.40. Two beams and two
flares: 0.3–0.6 ms. By day, or at night with no beam on and no flare burning, nothing is drawn: 0.

**Follow-ups.** A ship cutting a beam (the night-lights lane's searchlight shadow maps would give it: the beam's samples
reading the lamp's map); the flares' light inside the clouds' march instead of the patch on their base; the screens'
own lighting of the flares' light (the gas takes four flashes only); a second view's halo is drawn at its own size but
shares the uniform with the main view's frame.

**The halo's band (2026-10-21).** A "blocky band" in a star shell's halo was looked for and not found in the star shell
views: the halo's profile is smooth and the same in every direction but the height's, the bloom draws no square about
it, and the reach leaves no edge there (`native/docs/lighting/lights.md`, "Lamp glints and salvo smoke", for what was tried).

## Lights at range

`shaders/sprite.wgsl` (flashes, flame jets, sparks, grains, embers: all light added to the scene) draws a sprite
at least `LEAST_PIXELS` (1.5) pixels across, its light divided by the area it gained, so the light it adds is the
same: a flash smaller than a pixel (a muzzle flash 20 km off at the bare eye's lens is a third of one) no longer falls
between the pixels' samples and is lost or flickers under the temporal filter; it shows as a point. A sprite laid
flat on the sea keeps its size. By day a 38 cm turret's flash 20 km off is still only a faint point at 1×
(a few levels over the sky); through 8–16× its fireball and the gun smoke after it read plainly
(`.build/native/long-range/after-clear-b0/20km-flash-*.png`), and the smoke bank stays in sight for several seconds.
