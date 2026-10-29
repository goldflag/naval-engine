//! The looks for smoke, fire and explosions: the game's (the `flipbook` and `shadows` looks since 2026-10-04) and the
//! trial looks compared beside it, each switched on or off at run time.
//!
//! `naval-client` reads [`Trial::ENV`], a comma-separated list of names that changes the game's look ([`Trial::GAME`]:
//! `flipbook`, `views`, `merge`, `shadows`, `shock`, `funnel-puffs`, `column-cover` and `splash-cover` on, everything
//! else off): `warp`, `volume`, `funnel-plume` and `bright-flames` switch a look on, and a name with a leading minus
//! switches it off (`-flipbook` draws the effects from before the flipbook look, `-views` its smoke from one side only,
//! `-merge` its puffs layered one over another, `-shadows` without the smoke's shadows, `-shock` without the blast
//! waves, `-funnel-puffs` the funnels' billows and ribbon, `-column-cover` a fire's flames through its own column's
//! smoke, `-splash-cover` the smoke over the splash columns in front of it). The CPU half reads
//! [`crate::CombatEffects::trial`]; shaders read `lighting.trial` (`common.wgsl`). A layer without the flipbooks'
//! simulations (`EffectsLayer::set_flipbooks`) draws as if `flipbook` were off.
//! `bun native/tools/effects-trial.ts` films the same moments with any of them.

/// Which looks are on. [`Trial::default`] is every look off: the effects from before the flipbook look, which a
/// layer draws until its host hands it the flipbooks; the game's is [`Trial::GAME`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Trial {
    /// Still puffs rolled in the sprite shader along a flow field.
    pub warp: bool,
    /// Puffs, flames and fire columns drawn from the frames of baked fluid simulations, blended by their motion
    /// vectors. The game's look.
    pub flipbook: bool,
    /// With `flipbook`: a fireball's smoke seen from the side the eye is on. Each puff keeps a heading of its own and
    /// shows the simulation from the two baked sides nearest the eye's bearing on it, blended (`flipbook.rs`,
    /// [`crate::flipbook::SIDE_BOOKS`]), so circling it turns the smoke instead of the smoke turning with the eye. The
    /// game's look (since 2026-10-13). Off: every puff shows the one side, mirrored by its seed.
    pub views: bool,
    /// The gas's and the soot's puffs drawn as one body of smoke instead of layered one over another: added up in a
    /// buffer of their own whatever their order, each weighed by what the smoke in front of it lets through, and laid over the scene
    /// once (`EffectsLayer`'s merged smoke). Layered, a puff in front shows its whole outline over the puffs behind and
    /// two puffs that swap places as the eye or the smoke moves swap at once; merged, the nearer smoke shows most and
    /// the change is gradual. The game's look (since 2026-10-14), drawn where the scene's backdrop depth is read
    /// (`DepthSource::Backdrop`, the client's).
    pub merge: bool,
    /// Smoke shaded by the smoke between it and the sun, and its shadow on the ships, the sea and everything else that
    /// takes the sun's shadows. The game's look (since 2026-10-04), drawn while the sun's shadow maps are.
    pub shadows: bool,
    /// Ray-marched volumes for the largest bodies: a magazine's cloud, a burning ship's column.
    pub volume: bool,
    /// With `flipbook`: funnel exhaust as one chain of links of the simulated plume in place of the billows and
    /// ribbon. Off in the game.
    pub funnel_plume: bool,
    /// With `flipbook`: funnel exhaust as puffs of the gun smoke's simulated smoke laid along the plume, merged with
    /// the other smoke, in place of the billows and ribbon (`funnel.rs`, `draw_puffs`). The game's look (since
    /// 2026-10-20). Off (or without the flipbooks): the billows and the ribbon. `funnel-plume` wins over it.
    pub funnel_puffs: bool,
    /// The blast waves of guns and explosions (`crate::shock`): the front's bend of the scene behind it and the cloud
    /// that condenses behind it. The game's look (since 2026-10-12).
    pub shock: bool,
    /// A fire's flames, bodies of burning gas and embers fade behind the smoke of the fires' columns in front of them
    /// (the columns in the smoke-occlusion map: `EffectsLayer`'s `column_occluder_pipeline`, `common.wgsl`'s
    /// `columns_through`). The game's look (since 2026-10-24). Off: they shine through all of it, as they drew after it.
    pub column_cover: bool,
    /// The smoke, fire and glows fade behind the splash columns in front of them (the columns' cover, which their
    /// composite lays over the scene before the effects draw: `EffectsLayer::set_splash_cover`, `common.wgsl`'s
    /// `splashes_through`). The game's look (since 2026-10-25). Off: they draw over the columns, as they drew after
    /// them.
    pub splash_cover: bool,
    /// After dark a fire's flames a stop brighter than the game's, their light on the scene as the game's
    /// (`crate::combat::BRIGHT_FLAMES_AT_NIGHT`). Off in the game (2026-10-25: for Bill to pick between the two).
    pub bright_flames: bool,
}

impl Trial {
    /// The environment variable `naval-client` reads the list from.
    pub const ENV: &str = "NAVAL_EFFECTS_TRIAL";

    /// The game's look: the flipbook's explosions, flames and fire columns, its smoke seen from every side and merged;
    /// the funnels' smoke as the flipbook's puffs; the smoke's shadows; the blast waves.
    pub const GAME: Trial = Trial {
        warp: false,
        flipbook: true,
        views: true,
        merge: true,
        shadows: true,
        volume: false,
        funnel_plume: false,
        funnel_puffs: true,
        shock: true,
        column_cover: true,
        splash_cover: true,
        bright_flames: false,
    };

    const NAMES: &str = "warp, flipbook, views, merge, shadows, volume, funnel-plume, funnel-puffs, shock, \
                         column-cover, splash-cover or bright-flames, each optionally with a leading - to switch it \
                         off (flipbook, views, merge, shadows, shock, funnel-puffs, column-cover and splash-cover are \
                         on unless -flipbook, -views, -merge, -shadows, -shock, -funnel-puffs, -column-cover or \
                         -splash-cover)";

    /// The game's look changed by a comma-separated list: a name switches its look on, `-name` off; an unknown name
    /// is the error.
    pub fn parse(list: &str) -> Result<Self, String> {
        let mut trial = Self::GAME;
        for item in list
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            let (name, on) = match item.strip_prefix('-') {
                Some(name) => (name.trim(), false),
                None => (item, true),
            };
            let look = match name {
                "warp" => &mut trial.warp,
                "flipbook" => &mut trial.flipbook,
                "views" => &mut trial.views,
                "merge" => &mut trial.merge,
                "shadows" => &mut trial.shadows,
                "volume" => &mut trial.volume,
                "funnel-plume" => &mut trial.funnel_plume,
                "funnel-puffs" => &mut trial.funnel_puffs,
                "shock" => &mut trial.shock,
                "column-cover" => &mut trial.column_cover,
                "splash-cover" => &mut trial.splash_cover,
                "bright-flames" => &mut trial.bright_flames,
                other => {
                    return Err(format!(
                        "unknown effects look {other:?} in {}={list:?}: {}",
                        Self::ENV,
                        Self::NAMES
                    ));
                }
            };
            *look = on;
        }
        Ok(trial)
    }

    /// Whether any look but the game's is on, or one of the game's is off.
    pub fn differs_from_game(self) -> bool {
        self != Self::GAME
    }

    /// The looks on, comma-separated (`none` for none).
    pub fn names(self) -> String {
        let names: Vec<&str> = [
            (self.warp, "warp"),
            (self.flipbook, "flipbook"),
            (self.views, "views"),
            (self.merge, "merge"),
            (self.shadows, "shadows"),
            (self.volume, "volume"),
            (self.funnel_plume, "funnel-plume"),
            (self.funnel_puffs, "funnel-puffs"),
            (self.shock, "shock"),
            (self.column_cover, "column-cover"),
            (self.splash_cover, "splash-cover"),
            (self.bright_flames, "bright-flames"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
        if names.is_empty() {
            "none".to_owned()
        } else {
            names.join(",")
        }
    }

    /// `lighting.trial` as the shaders read it: x warp, y flipbook, z shadows, w volume, each 1 when on, y 2 with
    /// `views` too (the funnels' looks, the merged smoke and the blast waves are the CPU's alone).
    pub fn uniform(self) -> [f32; 4] {
        let mut uniform =
            [self.warp, self.flipbook, self.shadows, self.volume].map(|on| f32::from(u8::from(on)));
        if self.flipbook && self.views {
            uniform[1] = 2.0;
        }
        uniform
    }
}

/// Sampled textures the gas material binds in its fragment stage, every look's included (they are bound whether or
/// not the look is on): a device needs at least this many a stage to build the effects.
pub const SAMPLED_TEXTURES: u32 = 18;

/// The device limits the effects ask for. The gas material's fragment stage binds 14 sampled textures (the clouds'
/// cover and the splash columns' two among them), warp adds 1 and flipbook 3 (18, over WebGPU's default of 16); the
/// smoke shadows' 2 are read in the vertex stage. Kept above the default (up to 24) for the headroom the other layers'
/// materials use. The fleet binds every preset's gun covers' stored shapes as one storage buffer (naval-ships'
/// `fleet.rs`), over WebGPU's default 128 MiB binding with a mixed fleet of battleships, so buffers and storage bindings
/// may be as large as the adapter allows.
pub fn limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    let adapter_limits = adapter.limits();
    wgpu::Limits {
        max_sampled_textures_per_shader_stage: adapter_limits
            .max_sampled_textures_per_shader_stage
            .min(24),
        max_storage_buffer_binding_size: adapter_limits.max_storage_buffer_binding_size,
        max_buffer_size: adapter_limits.max_buffer_size,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_named_is_the_games_look() {
        assert_eq!(Trial::parse(""), Ok(Trial::GAME));
        assert_eq!(Trial::parse(" , "), Ok(Trial::GAME));
        const { assert!(Trial::GAME.flipbook && Trial::GAME.shadows && !Trial::GAME.funnel_plume) };
        const { assert!(Trial::GAME.views && Trial::GAME.merge && Trial::GAME.funnel_puffs) };
        const { assert!(Trial::GAME.shock && Trial::GAME.column_cover && Trial::GAME.splash_cover) };
        assert!(!Trial::GAME.differs_from_game());
    }

    #[test]
    fn a_list_switches_on_what_it_names_and_off_what_it_minuses() {
        let trial = Trial::parse("-shadows").unwrap();
        assert!(trial.flipbook && !trial.shadows && !trial.warp && !trial.volume);
        assert_eq!(trial.uniform(), [0.0, 2.0, 0.0, 0.0]);
        // One side of the smoke: the flipbook as it was before `views`; and views are nothing without the flipbook.
        assert_eq!(
            Trial::parse("-views").unwrap().uniform(),
            [0.0, 1.0, 1.0, 0.0]
        );
        assert_eq!(
            Trial::parse("-flipbook").unwrap().uniform(),
            [0.0, 0.0, 1.0, 0.0]
        );
        assert!(!Trial::parse("-merge").unwrap().merge);
        let off = Trial::parse(
            "-flipbook,-views,-merge,-shadows,-shock,-funnel-puffs,-column-cover,-splash-cover",
        )
        .unwrap();
        assert_eq!(off, Trial::default());
        assert!(!Trial::parse("-funnel-puffs").unwrap().funnel_puffs);
        assert!(!Trial::parse("-shock").unwrap().shock);
        assert!(!Trial::parse("-column-cover").unwrap().column_cover);
        assert!(!Trial::parse("-splash-cover").unwrap().splash_cover);
        assert!(Trial::parse("bright-flames").unwrap().bright_flames && !Trial::GAME.bright_flames);
        assert_eq!(off.names(), "none");
        let plume = Trial::parse("funnel-plume, warp").unwrap();
        assert!(plume.flipbook && plume.funnel_plume && plume.warp);
        assert_eq!(
            plume.names(),
            "warp,flipbook,views,merge,shadows,funnel-plume,funnel-puffs,shock,column-cover,splash-cover"
        );
        // The last word on a look wins.
        assert!(Trial::parse("-flipbook,flipbook").unwrap().flipbook);
        let error = Trial::parse("flipbok").unwrap_err();
        assert!(error.contains("-flipbook"), "{error}");
        assert!(Trial::parse("-nothing").is_err());
    }
}
