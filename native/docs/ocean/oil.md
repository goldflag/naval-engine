# Oil slicks

Index: [README.md](README.md).

## A leaking hull's slick

A hull losing oil from holed bunkers (the fuel lane's `ShipVisual::oil_leak_m3s`, m³/s, `naval_sim::bunkers`) trails a
slick (native only; branch `internals-slick`): `src/oil.rs` lays it, `shaders/oil.wgsl` shades it. Bismarck after the
Denmark Strait left one her shadowers followed.

- **Laid along her track.** Each hull's trail (`WakeFoam`) keeps an `OilSlick`, carried, re-laid and dropped with it.
  While she leaks (the packet's rate, from a litre a second) she lays a patch every 24 m of track, or every 4 s when she
  barely makes way, where her oil goes over the side as the patches see it: 0.3 of her length aft of her middle, about
  her after body, 0.45 of her beam either side. A patch's film is her oil per area of track: her leak over twice that
  breadth and her speed through the water (at least 2 m/s), so 0.3 m³/s from a Bismarck at 21 m/s lies 0.45 mm thick
  and a litre a second 1.5 µm. Patches reach 2.5 spacings along the track either way, so the film where they meet is
  within a few percent of their own (shorter ones beaded the slick into bands of colour), and a head patch at her source
  is painted at every rasterisation, so the slick keeps up with her.
- **As it ages.** A patch's core spreads 1.5 m a √s across her track (half that along it; her other patches fill in
  along it, so only its breadth thins it), its oil goes into the sheen with a 300 s time constant and off the water with
  the simulation's two hours (`SLICK_SECONDS`). The sheen about it, a film of 0.04 µm (less where the patch has too
  little oil to cover it), spreads 4 m a √s: 370 m across after half an hour. A patch lasts 30 minutes
  (`OIL_LIFETIME`, the last quarter fading it) and drifts downwind at 3.5 % of the sea's wind (the gale streaks' surface
  drift). From a large leak the core is metallic a quarter of an hour on and a sheen after half; from a small one it
  goes to the sheen within twenty minutes.
- **The atlas.** 3 × 3 tiles of 512² `Rg8Unorm` (4.5 MB on every tier), one slick each: the focus's, then, when there
  are more than nine, those nearest the camera (Codex review): red the film on a 2,048 m square centred on the slick's
  newest oil (4 m texels), green on an 8,192 m
  square framed on the slick (16 m texels, some six minutes of a full-speed run). Both follow the slick, not the hull:
  one she has left stays where it lies however far she goes. A texel holds the film's logarithm from 10 nm to 1 mm (5 %
  a step), so the painter's max blending keeps the thickest film and its profile falls through the colours to its edge:
  a thick core, metallic and rainbow rims, a sheen outside. The rows ride the `Wake` uniform (`oil`, `oil_reach`,
  `oil_flow`, `oil_tiles`), the texture is the surface's group 1 binding 16. A slick is rasterised at twice the trail's
  interval while she leaks and once a second after.
- **The look** (`shaders/oil.wgsl`). The film is read through a warp of up to 7 m, and its log thickness moved by
  windrows along the wind (bands 26 m apart and seven times as long, riding the drift) and patches 70 m across (up to
  16 % of itself, a thin film more). Its reflectance is Airy's for a film of fuel oil (n 1.48) on water, at the view's
  angle and both polarisations, the interference washed out past its second or third order by the phase's spread over a
  channel's band and the film's unevenness (9 %): from the clean sea's 2 % at normal incidence to 6 % at 0.1 µm, in its
  colours where it is a few tenths of a micron, and 4 % once it is thick. It multiplies the surface's own reflectance, so
  at grazing angles, where both reflect nearly everything, the film shows only by what it stills. Under it the water's
  light crosses it twice through heavy fuel oil's absorption (1.2, 2.4 and 4.5 × 10⁴ /m, red to blue: brown, and black
  past a tenth of a millimetre), the oil's own dark brown taking what it stops. Any film stills the short waves as the
  wake's slick does (`calm` 0.6 for a sheen to 0.85 thick: the ripples, the glitter's spread, the sky's blur) and damps
  their breaking (the whitecaps, the windrows' foam and the bubbles under them, by 0.8 of its calm). Thick oil lies in
  the windrows and a thin film in the gaps between them (about a third of the slick, up to 0.6 of its log-thickness
  gone), so a dark slick shows its faint rainbow in patches (the Bonn code's discontinuous true colour), and the colour
  keeps 60 % of its saturation (`OIL_COLOUR`): all of it drew a crisp rainbow line along the rim. The oil's own albedo is
  0.5, 0.34 and 0.22 % (red to blue): a percent of red drew the slick rust-red from the air.
- **Only where there is oil.** The surface builds its four pipelines again with `OIL` (two on a tier tracing no
  reflections; `OceanSurface::oil_pipelines`, `sky_oil_pipelines`) and draws them on frames whose wakes have a slick in a
  slot (`ShipWake::oil_shown`, `OceanSurface::oil`): a frame without one draws the surface as it was, the oil compiled
  out. The fragment stage binds 19 sampled textures with the atlas (the client asks for up to 24; WebGPU's default is 16,
  which the sea passed before this).

**Cost** (Apple M5 Pro, 2026-10-20, the High preset at 1728 × 1030, `dev-client`, load average 20–24 from other
sessions; `shot.ts --settle 16 --oil-ab 2`: the water and sky pass's GPU time with the slick drawn and not, alternating
at one frozen moment, 32 frames a side): a Bismarck three minutes into a 0.3 m³/s leak, one slick of 112 patches (318
stamps), +0.24 ms of 3.54 close astern, +0.36 of 3.28 low into the sun, +0.10 of 3.95 from 1.2 km up on her quarter and
+0.32 of 3.99 from 1.5 km straight down; the whole frame's medians moved within their noise (±0.7 ms). A second run at
load 34–44, ten minutes into a 0.1 m³/s leak at 10.5 m/s (218 patches, 530 stamps): +0.41 ms of 4.48 close on her
quarter, +0.80 of 3.77 from 1.8 km up over the whole slick and +0.41 of 4.17 straight down. Nothing on frames without
oil.

**Captures** (`.build/native/oil/`, each beside its `-no-oil` twin from the same moment): `noon-*.png`, three minutes
of a 0.3 m³/s leak at 14 m/s under the default cloud (close astern, low into the sun, the high quarter, straight down);
`long-*.png`, ten minutes of a 0.02 m² hole at 7 m/s in clear weather (`close` her quarter 70 m up, `air` 1.8 km up over
the 6 km slick, `down` over its four-minute-old oil). Near her the wake's turquoise water is a dark brown-black film
under the foam with a faint rainbow at its rim; further back it breaks into patches along the windrows and fades to a
smooth band long after the wake's slick is gone; straight down into the noon sun it is a smooth bright streak in the
glitter's core and a dark one outside it, as slicks show in sun glint. The atlas's repaint (a few hundred to a few thousand small quads, 10 Hz while she leaks near the camera,
1 Hz after) was not measured apart; the trail's 500–600 stamps repaint in 0.14–0.27 ms ([wakes.md](wakes.md), "Cost").

**Staging it.** The debug RPC's `hole_bunker` holes a bunker of a manual battle's ship (a harness seam in
`naval_protocol::local::LocalBattle`, beside the console's `change_wind`; not a `Command`), `shot.ts --hole-bunker`
stages it at tick 0 and `--oil-ab` times it (`native/docs/harness.md`, "Staging a shot"); the RPC's `water {oil: false}` draws the
sea without its slicks (`Ocean::draw_oil`) and `water {}` answers the slicks' slots, stamps and patches. The ship needs
bunker data in her published model.

## Her oil after she goes, and burning

A lost ship's own oil is a large part of what she leaves on the sea: Hood went down with some 4,600 t aboard and five
minutes on "one large red raft and a considerable amount of wreckage amidst a huge patch of oil" lay where she went
(Sunderland Z/201, AIR 15/415); Barham left "a bubbling, oily-looking patch" (Cunningham). Arizona still leaks a few
litres a day; how much comes up in the first minutes is not recorded for any warship.

- **It comes up** (`oil::wreck_leak_m3s`): once she has gone under (`naval_effects::loss`, all of her under the sea),
  the host adds to her slick's leak a tenth of her bunkers' oil (`WRECK_OIL_SHARE`: the tanks the loss ruptured; an
  estimate), coming up with a time constant of three minutes (`WRECK_OIL_SECONDS`). Her bunkers are her definition's
  (`naval_effects::ships::bunker_oil_m3`), else an eighth of her displacement (a warship's fuel load: Bismarck 7,400 t
  of about 50,000, King George V 3,700 of 42,000, Fubuki 500 of 2,100: navypedia). It is laid as any leak is, where she
  lies, so it lies thick (a Bismarck's first minutes some 60 mm under her, spreading) and drifts and weathers as the
  rest; the slick stays with her trail, which stays while her wreck is drawn.
- **It burns, seldom** (`OilSlick::ignite`, `fires`, `burn`, `sea_holds_a_burn`; since 2026-10-21, the second
  pass). In the war's record warship fuel oil seldom burned on the sea after a loss: Barham's, Juneau's (a magazine),
  Bismarck's, Repulse's and Prince of Wales's, Scharnhorst's, Indianapolis's, Yamato's and Royal Oak's survivors swam
  in thick oil that did not burn (their accounts; Juneau's oil "at least two inches thick"). It burned where a ship
  still burning held her fresh oil against her in calm water: Arizona in harbour, for one to two days, fed by her wreck
  (her reports and her neighbours'); Fuso in Surigao Strait, some three hours, in "multiple burning oil pools" (Tully
  and Lundgren, 2024); Blücher as she rolled over (Haarr and Melien, 2023); Liscome Bay's "burning patches" with her
  aviation petrol. At Hood's site, "a flicker of flame and smoke on the water" (Nixon, Daily Express, 31 May 1941). The
  oil-spill research agrees: heavy fuel oil needs a sustained flame and calm water, waves above a metre rule a burn out
  (ARPEL Report 28's decision chart, Fingas 2007; Opstad and Guénette, *Fire Safety Science* 6, 2000), and a slick
  spread on open water is too thin to light. So a hull still burning above the water within a minute of going under
  (`naval_effects::loss::FIRE_MEMORY_SECONDS`: her own fire going down with her) lights her oil within half her length
  of where she went, only in a sea that holds a burn (waves of a metre at most, wind of 10 m/s at most:
  `BURN_WAVES_M`, `BURN_WIND_MPS`), and thick oil coming up there lights for two minutes after; a sea that gets up past
  that puts out what burns and what would still have lit (`OilSlick::put_out`). A magazine's flash alone lights nothing (until 2026-10-21 any magazine lit it, in any sea). Heavy fuel oil lights only where it lies some
  10 mm thick
  (`IGNITE_M`; Buist, Interspill 2004: Bunker C and IFO 380; Fingas, ARPEL report 28, 2007: heavy oils "require longer
  heating times and a hotter flame"), so it burns over her and not far off; it burns down at a millimetre a minute
  (`BURN_M_PER_S`, Fingas: heavy oils) and goes out at about a millimetre of residue (`RESIDUE_M`, Buist); the fire
  takes thick neighbours within 10 m of a burning core. A patch's burned film comes off its core, so what burned is
  left a millimetre thick and dark. The burning patches go to the combat effects (`naval_effects::sea_fires`), which
  draw their flames and black smoke in patches where the oil lies thick (`native/docs/combat-effects/ship-loss.md`, "A ship's loss"). A
  battleship's oil lit as she goes burns for some ten minutes, the fire going out as the oil coming up thins below what
  lights. Whether the game should draw it at all is the owner's to decide: with this rule it shows only in a calm sea
  after a burning ship goes down.
- **The boil** ("Sinkings", `wake/impulses.rs`; [wakes.md](wakes.md), "The sea answers what strikes it") lies over her keel's footprint
  on the sea, its length times the cosine of her trim, and where she goes under is an irregular cluster of white domes
  the size of that footprint (since 2026-10-21; before, one box of white water along her length whatever her trim). No
  measured size of a sinking ship's boil was found; the accounts give "a swirl & a rush" and "a few bubbles" (Triumph, 1915, Cicognani's diary), a surge of air that shot
  Briggs to the surface (Hood), "a bubbling, oily-looking patch" (Barham). After the boil the patch is quiet: the oil
  coming up stills its ripples (the film's `calm`), and her flotsam surfaces in it over the first minute
  (`naval_effects::flotsam`).

Checked by `oil.rs`' `a_wrecks_oil_comes_up_where_she_went_and_burns_where_it_lies_thick` (a tenth of 7,000 m³ over
minutes, thick and burning a minute on, burned down to its residue, out within half an hour; unlit, never burning).

**Not done.** A ship that leaves the scene (out of the drawn hulls) takes her slick with her trail. The wide
square holds some six minutes of a full-speed run, so the oldest oil of a long straight run is cut at its edge (faded
over its outer 200 m, as the trail's slick is at its square's). The simulation's own record of where she leaked (`Bunkers::slick`, checkpoints only) does
not seed the slick of a battle restored from one, nor of a theatre ship that comes aboard (her trail's `lay_trails`
lays it at her present leak). Seen from under the sea the film draws nothing. A fire's light glints on it as on water.
A burning patch's state is not saved: a restored battle's slick is laid unlit.
