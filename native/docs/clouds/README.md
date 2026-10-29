# Clouds and weather (`native/crates/naval-clouds`)

| Question | Page |
| --- | --- |
| What does the crate port from takram, and where does it depart? How do other lanes drive and read it, what do the tiers trim, how is a cloud lit (broken cloud, bright tops, the moon and lightning), how are its grain and near detail kept down, what does it cost, how do I run the stills and timings, and what is open? | [The cloud march](march.md) |
| Where are the clouds laid (the simulation's cloud anchors)? What does each weather draw, how does weather change and the sun move through a battle, how is rain drawn, and what does aircraft spotting share with the clouds? | [Weather](weather.md) |

The clouds are a port of **@takram/three-clouds** (packages/clouds of
https://github.com/takram-design-engineering/three-geospatial, MIT; notices in `naval-clouds/THIRD_PARTY.md`) to
wgpu: four cloud layers over the curved planet, takram's weather and shape textures (the detail noise is the crate's
own since 2026-10-04, [near detail](march.md#near-detail)), a ray march lit through
the sky's atmosphere (takram's own Bruneton model, `native/docs/sky.md`), Beer shadow maps (BSM) in cascades, temporal
upscaling, light shafts and haze. They replaced the native port of the web game's clouds (2026-10-02). The weather is still the game's, ported from the web game's `src/game/sky/weather` (historical): near rain, splashes and
lightning (the bolt, the scene flash, thunder); since 2026-10-20 the rain is drawn as a camera records it and its
distance is its share of the haze ([rain](weather.md#rain-as-a-camera-records-it)). The crate is portable
(`cargo check -p naval-clouds --target wasm32-unknown-unknown`) and builds without the simulation (`naval-sim`,
`naval-protocol`): the layers, the weathers' profiles, the weather texture and the anchors' blend it shares with the
simulation are `crates/naval-cloud-field`'s, and the game lays a simulated field on its cloud anchors through
`native/crates/naval-clouds-sim` ([the cloud anchors](weather.md#the-cloud-anchors)). Outputs go to `.build/native/clouds/`.

History (the lane's brief, measurements before and after each change, the cost tables, the web-era re-sync):
[native-clouds-history.md](../../../docs/archive/native-clouds-history.md).
