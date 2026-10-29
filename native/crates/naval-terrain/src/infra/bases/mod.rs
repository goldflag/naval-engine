//! Generated wartime bases (native/docs/terrain/infra.md, "Generated bases"): a catalog of bases at roughly the right
//! place
//! (`assets/bases/bases.v1.json`, embedded), a site fitted to the ground about each ([`site::fit`]: the flattest dry
//! ground long enough for its runway, or today's runway where the map has one), and its nation's template laid out on
//! it ([`Template`]: [`strip::us_pacific_strip`], a coral or Marston-mat runway, its taxiway, dispersal loops with
//! revetted hardstands and a camp of Quonset huts and tents; [`ijn::ijn_solomons_field`], a Japanese navy field's wide
//! rough runway, taxiways winding into the jungle to horseshoe revetments, a camp scattered under the trees), computed
//! once per base in metres about its point ([`strip::Plan`]) and drawn by each chunk of the infrastructure that holds a
//! piece of it ([`draw::draw`]). Presentation only: the simulation knows nothing of it. The famous fields are not
//! generated (`infra::period::FIELDS` draws them), unless the catalog has one in years it lists (Munda, Japanese).
//!
//! Every choice is seeded from the base's id ([`BaseEntry::seed`]), so a base is the same on every visit.

pub mod draw;
pub mod ijn;
pub mod luftwaffe;
pub mod raf;
pub mod site;
pub mod strip;

use std::sync::OnceLock;

use serde::Deserialize;

pub use draw::Placed;
pub use site::{Site, SiteInput};
pub use strip::Plan;

const BASES_JSON: &str = include_str!("../../../../../../assets/bases/bases.v1.json");

/// What a base is (`kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BaseKind {
    Airfield,
}

/// A base's size (`size`): its runway and how much stands about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SizeClass {
    /// A fighter strip: 4,000 ft of Marston mat (1,220 m) on coral.
    FighterStrip,
    /// A bomber field: 6,500 ft of coral (1,980 m) with its shoulders.
    BomberField,
}

impl SizeClass {
    /// The runway's length (m).
    pub fn length(self) -> f32 {
        match self {
            Self::FighterStrip => 1220.0,
            Self::BomberField => 1980.0,
        }
    }

    /// The runway's paved half width (m): 150 ft wide.
    pub fn half(self) -> f32 {
        23.0
    }

    /// Each shoulder's width (m): a bomber field's 75 ft of coral either side, a fighter strip's few metres of coral
    /// beside its mat.
    pub fn shoulder(self) -> f32 {
        match self {
            Self::FighterStrip => 7.0,
            Self::BomberField => 12.0,
        }
    }

    /// How far from its point the runway's middle may be fitted (m).
    pub fn search_m(self) -> f32 {
        match self {
            Self::FighterStrip => 1500.0,
            Self::BomberField => 2000.0,
        }
    }

    /// How far from its point anything of the base may stand (m): its search, half its runway and its camp beyond.
    pub fn reach_m(self) -> f32 {
        self.search_m() + 0.5 * self.length() + 900.0
    }
}

/// The template a base is laid out by: its nation's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    /// `us-pacific-strip` ([`strip::us_pacific_strip`]).
    UsPacificStrip,
    /// `ijn-solomons-field` ([`ijn::ijn_solomons_field`]).
    IjnSolomonsField,
    /// `raf-station` ([`raf::raf_station`]): a Class A bomber station, or a grass fighter field.
    RafStation,
    /// `luftwaffe-field` ([`luftwaffe::luftwaffe_field`]): a Fliegerhorst, or a Feldflugplatz.
    LuftwaffeField,
}

impl Template {
    /// A nation's (`nation`): Japan's the Japanese navy's field, any other the American strip.
    pub fn of_nation(nation: &str) -> Self {
        match nation {
            "Japan" => Self::IjnSolomonsField,
            "UK" => Self::RafStation,
            "Germany" => Self::LuftwaffeField,
            _ => Self::UsPacificStrip,
        }
    }

    /// Its runway for a base of `size`: length, paved half width and each shoulder's width (m). A Japanese field of
    /// 1943 in the Solomons: 1,100 to 1,350 m of rolled coral and gravel some 55 m wide (Kara's 4,000 by 280 ft
    /// cleared, Kahili's 4,700 by 300).
    pub fn runway(self, size: SizeClass) -> (f32, f32, f32) {
        match self {
            Self::UsPacificStrip => (size.length(), size.half(), size.shoulder()),
            Self::IjnSolomonsField => match size {
                SizeClass::FighterStrip => (1100.0, 25.0, 3.0),
                SizeClass::BomberField => (1350.0, 27.0, 3.0),
            },
            // A Class A bomber station's main runway, 2,000 yd by 50 yd; a fighter field's grass, its landing run
            // some 90 m wide.
            Self::RafStation => match size {
                SizeClass::FighterStrip => (1100.0, 45.0, 0.0),
                SizeClass::BomberField => (1830.0, 23.0, 0.0),
            },
            // A Fliegerhorst's concrete runway, 1,500 by 60 m; a Feldflugplatz's grass strip.
            Self::LuftwaffeField => match size {
                SizeClass::FighterStrip => (1000.0, 40.0, 0.0),
                SizeClass::BomberField => (1500.0, 30.0, 0.0),
            },
        }
    }

    /// Whether its bases are temperate (Europe's: their cleared ground mown grass).
    pub fn temperate(self) -> bool {
        matches!(self, Self::RafStation | Self::LuftwaffeField)
    }
}

/// `site`'s base laid out by `input`'s template.
pub fn lay_out(site: Site, input: &SiteInput<'_>) -> Plan {
    match input.template {
        Template::UsPacificStrip => strip::us_pacific_strip(site, input),
        Template::IjnSolomonsField => ijn::ijn_solomons_field(site, input),
        Template::RafStation => raf::raf_station(site, input),
        Template::LuftwaffeField => luftwaffe::luftwaffe_field(site, input),
    }
}

/// One base of the catalog.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BaseEntry {
    pub id: String,
    pub name: String,
    pub kind: BaseKind,
    pub nation: String,
    pub lat: f64,
    pub lon: f64,
    /// The first and last years it stood (both drawn).
    pub from: u16,
    pub to: u16,
    pub size: SizeClass,
    /// A hint for the runway's heading (degrees clockwise from true north).
    #[serde(default)]
    pub heading_deg: Option<f32>,
    /// Its runways' surface where not its template's (`gravel`: Kaldaðarnes's).
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

impl BaseEntry {
    /// The template it is laid out by.
    pub fn template(&self) -> Template {
        Template::of_nation(&self.nation)
    }

    /// Whether it stood in `year`.
    pub fn stands(&self, year: u16) -> bool {
        (self.from..=self.to).contains(&year)
    }

    /// The seed of its every choice: a hash of its id.
    pub fn seed(&self) -> u32 {
        self.id.bytes().fold(2_166_136_261u32, |h, b| {
            (h ^ u32::from(b)).wrapping_mul(16_777_619)
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    version: u32,
    #[allow(dead_code)]
    about: String,
    bases: Vec<BaseEntry>,
}

/// The catalog, parsed once.
pub fn catalog() -> &'static [BaseEntry] {
    static CATALOG: OnceLock<Vec<BaseEntry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let file: CatalogFile =
            serde_json::from_str(BASES_JSON).expect("assets/bases/bases.v1.json parses");
        assert_eq!(file.version, 1, "assets/bases/bases.v1.json's version");
        file.bases
    })
}

/// The bases standing in `year`.
pub fn standing(year: u16) -> impl Iterator<Item = &'static BaseEntry> {
    catalog().iter().filter(move |b| b.stands(year))
}

/// A small deterministic generator (SplitMix64) for a base's choices.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u32, stream: u32) -> Self {
        Self((u64::from(seed) << 32 | u64::from(stream)) ^ 0x9e37_79b9_7f4a_7c15)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// 0 to 1.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// −1 to 1.
    pub fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    /// One of 0..n.
    pub fn pick(&mut self, n: usize) -> usize {
        ((self.unit() * n as f32) as usize).min(n.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_parses_and_its_ids_are_its_own() {
        let bases = catalog();
        assert!(bases.len() >= 6, "{}", bases.len());
        let mut ids: Vec<&str> = bases.iter().map(|b| b.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), bases.len(), "an id twice");
        for b in bases {
            assert!(b.from <= b.to, "{}", b.id);
            assert!(
                matches!(b.nation.as_str(), "US" | "Japan" | "UK" | "Germany"),
                "{}: no template for {}",
                b.id,
                b.nation
            );
            assert!(
                b.surface.as_deref().is_none_or(|s| s == "gravel"),
                "{}: surface",
                b.id
            );

            assert!((-90.0..=90.0).contains(&b.lat) && (-180.0..=180.0).contains(&b.lon));
            assert_ne!(b.seed(), 0);
            // Henderson Field is drawn as the period draws it, not generated.
            assert!(
                !crate::infra::period::FIELDS
                    .iter()
                    .any(|f| f.place.holds(b.lat, b.lon) && f.place.name.contains("Henderson")),
                "{} lies on Henderson Field",
                b.id
            );
        }
        assert!(standing(1943).count() >= 6);
        // Before 1942 only Europe's stood; before 1936 none.
        assert!(standing(1941).all(|b| b.template().temperate()));
        assert_eq!(standing(1935).count(), 0);
    }

    #[test]
    fn the_generator_is_the_same_for_the_same_seed() {
        let a: Vec<u64> = (0..4).map(|_| Rng::new(7, 1).next_u64()).collect();
        assert!(a.windows(2).all(|w| w[0] == w[1]));
        let mut r = Rng::new(7, 1);
        let mut s = Rng::new(7, 2);
        assert_ne!(r.next_u64(), s.next_u64());
        let mut r = Rng::new(3, 0);
        for _ in 0..1000 {
            let u = r.unit();
            assert!((0.0..1.0).contains(&u));
        }
    }
}
