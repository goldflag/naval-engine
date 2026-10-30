//! The buildings' colours by region and kind: palette entries the shader (`buildings.wgsl`'s `WALLS` and `ROOFS`)
//! turns into colours. 0-31 are authored, 32-247 a 6 × 6 × 6 colour cube ([`super::cube_entry`]) for OSM's colours and
//! cranes, 248-255 authored again ([`EXTRA_FIRST`]).

use super::{CUBE_FIRST, Kind, Period, Region, pick};

// Walls: 0 white render, 1 cream, 2 light concrete, 3 mid concrete, 4 red brick, 5 brown brick, 6 yellow brick, 7 pastel
// blue, 8 pastel green, 9 pastel pink, 10 ochre, 11 grey sheet metal, 12 dark timber, 13 bright white, 14 dark glass, 15
// sand, 16 salmon render, 17 warm yellow render, 18 whitewash, 19 light ochre, 20 peach, 21 light grey siding, 22 beige
// siding, 23 blue-grey siding, 24 Falu red, 25 blue sheet metal, 26 green sheet metal, 27 cream sheet metal, 28 blue
// glass, 29 green glass, 30 stone grey, 31 limestone; 248 harl, 249 purple-brown (klinker) brick, 250 red sandstone,
// 251 honey limestone.
// Roofs: 0 terracotta, 1 dark terracotta, 2 slate, 3 asphalt, 4 red iron, 5 green iron, 6 blue iron, 7 galvanised, 8
// rusted, 9 gravel, 10 pale membrane, 11 tar, 12 black, 13 brown tile, 14 concrete, 15 white, 16 orange terracotta, 17
// faded terracotta, 18 grey shingle, 19 brown shingle, 20 dark shingle, 21 copper patina, 22 lead, 23 red-brown tile, 24
// white membrane, 25 blue slate, 26 anthracite, 27 green-grey, 28 sand tile, 29 light galvanised, 30 gilt, 31 blue-grey
// lead; 248 reed thatch, 249 limestone screed, 250 smoked (ibushi) kawara, 251 dark glazed kawara.

/// The first of the authored entries after the colour cube (`buildings.wgsl`'s `WALLS_EXTRA`, `ROOFS_EXTRA`).
pub const EXTRA_FIRST: u8 = CUBE_FIRST + 216;
/// Walls: Scottish harl, grey roughcast render.
pub const HARL: u8 = EXTRA_FIRST;
/// Walls: purple-brown klinker brick, the Low Countries' and north Germany's.
pub const KLINKER: u8 = EXTRA_FIRST + 1;
/// Walls: red sandstone (Glasgow's tenements, Kirkwall's cathedral).
pub const RED_SANDSTONE: u8 = EXTRA_FIRST + 2;
/// Walls: Malta's honey-coloured globigerina limestone.
pub const HONEY: u8 = EXTRA_FIRST + 3;
/// Walls: charred or dark-stained boards (Japan's yakisugi and shitami-ita), a past period's only.
pub const CHARRED: u8 = EXTRA_FIRST + 4;
/// Roofs: reed thatch, Denmark's, Holland's and north Germany's farmhouses' (and in a past period the Pacific's sago
/// and pandanus leaf).
pub const THATCH: u8 = EXTRA_FIRST;
/// Roofs: a flat roof's pale, warm screed of limestone dust and lime, Malta's and North Africa's.
pub const SCREED: u8 = EXTRA_FIRST + 1;
/// Roofs: smoked (ibushi) kawara, darker and more silvered than [`KAWARA`]'s weathered grey.
pub const IBUSHI: u8 = EXTRA_FIRST + 2;
/// Roofs: glazed kawara in black and blue-black (yuyaku), the Hokuriku's and the San'in's, a town's newer houses'.
pub const GLAZED: u8 = EXTRA_FIRST + 3;

pub(super) const FLAT_ROOFS: &[u8] = &[9, 9, 11, 14, 10, 3, 24];
/// A warm, dry country's flat roofs: pale concrete, membranes and limewash, light gravel (no tar or asphalt).
pub(super) const PALE_FLAT_ROOFS: &[u8] = &[14, 10, 24, 15, 14, 9, 10, 24];
/// Egypt's flat roofs: dusty concrete, gravel and tan.
const DUSTY_FLAT_ROOFS: &[u8] = &[14, 9, 28, 14, 10, 14];
/// Malta's flat roofs: limestone screed on half, pale concrete and membranes.
const MALTA_FLAT_ROOFS: &[u8] = &[SCREED, 14, SCREED, 10, 24, SCREED, 15, SCREED];
/// North Africa's flat roofs: limewash and white membranes, limestone screed, pale concrete.
const AFRICA_FLAT_ROOFS: &[u8] = &[15, 24, SCREED, 14, 24, 10, SCREED, 15, 9];

/// Taiwan's flat roofs: grey concrete, green-grey waterproofing, dark felt, gravel, brown and red-brown heat tiles
/// (隔熱磚).
const TAIWAN_FLAT_ROOFS: &[u8] = &[14, 14, 27, 13, 3, 14, 27, 9, 14, 11, 23, 3];

/// A region's flat roofs.
pub(super) fn flat_roofs(region: Region) -> &'static [u8] {
    use Region::*;
    match region {
        Taiwan => TAIWAN_FLAT_ROOFS,
        Mediterranean | Aegean | Arid | Hawaii | Micronesia | Atoll => PALE_FLAT_ROOFS,
        Malta => MALTA_FLAT_ROOFS,
        NorthAfrica => AFRICA_FLAT_ROOFS,
        Nile => DUSTY_FLAT_ROOFS,
        _ => FLAT_ROOFS,
    }
}
/// Grey kawara (ibushi) tile: the colour cube's (102, 102, 102).
pub const KAWARA: u8 = CUBE_FIRST + 2 * 36 + 2 * 6 + 2;
/// Walls: Taiwan's darker facade tiles from the colour cube: grey (153, 153, 153), dark grey (102, 102, 102), tan
/// (204, 153, 102), dun (153, 153, 102), olive (102, 102, 51), brown (153, 102, 51), mauve (153, 102, 102) and rose
/// (204, 153, 153).
pub const TILE_GREY: u8 = CUBE_FIRST + 3 * 36 + 3 * 6 + 3;
pub const TILE_DARK: u8 = CUBE_FIRST + 2 * 36 + 2 * 6 + 2;
pub const TILE_TAN: u8 = CUBE_FIRST + 4 * 36 + 3 * 6 + 2;
pub const TILE_DUN: u8 = CUBE_FIRST + 3 * 36 + 3 * 6 + 2;
pub const TILE_OLIVE: u8 = CUBE_FIRST + 2 * 36 + 2 * 6 + 1;
pub const TILE_BROWN: u8 = CUBE_FIRST + 3 * 36 + 2 * 6 + 1;
pub const TILE_MAUVE: u8 = CUBE_FIRST + 3 * 36 + 2 * 6 + 2;
pub const TILE_ROSE: u8 = CUBE_FIRST + 4 * 36 + 3 * 6 + 3;

/// The walls' and roof's palette entries for a building of `kind` in `region` (`pitched` whether its roof slopes), by
/// two hashes; in a past `period` from [`past_colours`].
pub(super) fn colours(
    region: Region,
    kind: Kind,
    period: Period,
    pitched: bool,
    a: f32,
    b: f32,
) -> (u8, u8) {
    if period.is_past() {
        return past_colours(region.in_period(period), kind, pitched, a, b);
    }
    use Region::*;
    let (walls, roofs): (&[u8], &[u8]) = match kind {
        Kind::Industrial | Kind::Warehouse => match region {
            Tropical | Micronesia | Atoll => (&[11, 13, 2, 11, 27], &[7, 8, 7, 4, 29]),
            Arid | NorthAfrica | Nile | Malta => (&[15, 2, 13, 27, 11], &[14, 7, 29, 24]),
            // Brick warehouses and mills beside the sheds.
            British | NorthSea => (
                &[11, 4, 13, 5, 3, 2, 11, KLINKER, 27],
                &[7, 29, 3, 26, 14, 2, 7, 10],
            ),
            _ => (
                &[11, 13, 3, 2, 11, 25, 26, 27],
                &[7, 29, 3, 10, 7, 14, 6, 5],
            ),
        },
        Kind::Office => (&[28, 29, 14, 2, 13, 3, 28], flat_roofs(region)),
        Kind::Block | Kind::Tower => match region {
            Temperate => (&[0, 1, 2, 4, 3, 5, 14, 30, 31], &[2, 25, 13, 23, 26]),
            Mediterranean => (&[18, 0, 1, 17, 19, 20, 13, 31, 16], &[0, 16, 17, 1]),
            // Stockholm's, Oslo's and Helsinki's render in ochre, yellow, cream and pink, some brick, under black sheet,
            // copper and red.
            Nordic => (
                &[1, 17, 10, 0, 19, 9, 4, 2, 13, 20, 3],
                &[26, 21, 12, 2, 4, 0],
            ),
            Iceland => (&[2, 13, 3, 1, 0, 14, 7], &[4, 12, 2, 26]),
            American => (&[2, 4, 5, 13, 3, 1, 22], &[18, 20, 3]),
            Tropical => (&[0, 13, 1, 7, 2, 8], &[7, 8, 4]),
            Arid => (&[15, 1, 0, 13, 2, 31], &[14, 15, 9, 10]),
            West => (&[0, 1, 13, 15, 22, 31, 2, 19, 20, 18], &[18, 20, 0]),
            EastAsia => (&[2, 0, 13, 1, 22, 31, 21, 15, 2, 3], &[KAWARA, 26, 31]),
            // Taiwan's walk-ups and towers in small tiles: greys, beige, tan, salmon and brown, a white one now and
            // then.
            Taiwan => (
                &[
                    3, TILE_GREY, 30, 15, 22, TILE_TAN, 16, TILE_DUN, TILE_DARK, 2, 5, TILE_GREY,
                    3, TILE_ROSE, 13, TILE_MAUVE,
                ],
                &[6, 7, 29, 15],
            ),
            // Brick mansion blocks and council flats, concrete, render.
            British => (&[4, 5, 2, 0, 1, 3, 6, 4, 30, 5], &[2, 25, 26, 23]),
            // Sandstone and harled tenements under slate.
            Celtic => (
                &[HARL, 30, RED_SANDSTONE, 0, 2, 31, HARL, 1, 3],
                &[2, 25, 26, 2],
            ),
            NorthSea => (&[4, KLINKER, 5, 6, 2, 0, 4, 3, 1], &[0, 16, 26, 12, 23]),
            Florida => (&[0, 13, 1, 20, 15, 7, 13, 9], &[15, 16, 0, 18]),
            Malta => (
                &[HONEY, 31, 1, HONEY, 19, HONEY, 15, 13, HONEY, 0],
                &[0, 16],
            ),
            // Athens' polykatoikia: white, cream and pale ochre concrete.
            Aegean => (&[0, 18, 1, 13, 19, 2, 31, 15, 20, 1, 0], &[0, 16]),
            NorthAfrica => (&[0, 13, 18, 1, 19, 15, 2, 31, 20, 0], &[0, 16]),
            // Beige and ochre render, red brick between concrete frames.
            Nile => (&[15, 1, 19, 10, 2, 4, 5, 1, 3, 0, 20, 15], &[14]),
            // Waikiki's and Honolulu's white, cream and sand concrete towers and walk-ups.
            Hawaii => (&[0, 13, 1, 2, 15, 20, 31, 13, 0, 3, 22], &[7, 29, 18]),
            Micronesia => (&[0, 13, 1, 2, 20, 15, 31, 9, 7, 13, 3], &[7, 29, 4]),
            Atoll => (&[2, 0, 1, 13, 3, 7, 8, 15], &[7, 8, 29, 4]),
        },
        Kind::Church => match region {
            Temperate => (&[30, 31, 4, 5, 0, 30], &[2, 25, 2, 21, 22, 23]),
            // White timber churches under black, copper or red.
            Nordic => (&[13, 13, 0, 30, 4], &[12, 26, 21, 4, 12]),
            Iceland => (&[13, 0, 30, 13, 2], &[4, 12, 21, 2, 12]),
            Mediterranean => (&[31, 18, 1, 19, 30, 17], &[0, 16, 17, 0]),
            American => (&[13, 4, 0, 31, 5], &[18, 2, 20, 13, 21]),
            Tropical => (&[13, 0, 1, 18], &[4, 7, 0, 7]),
            Arid => (&[15, 31, 13, 1], &[14, 15, 0]),
            West => (&[13, 0, 18, 1, 31], &[0, 18, 20, 16]),
            // Temples and shrines: dark timber and white plaster under kawara and dark glazed tile, a copper roof.
            EastAsia => (&[12, 13, 0, 12, 30], &[KAWARA, 26, 12, 21, KAWARA]),
            // A temple's red walls and pillars under orange and green glazed tile, a church's white or brick.
            Taiwan => (&[4, 4, 16, 13, 30, 4, 1], &[16, 0, 16, 5, 1, 16, 30]),
            British => (&[30, 31, 30, 4, RED_SANDSTONE, 5], &[2, 22, 25, 2, 21, 23]),
            Celtic => (&[30, RED_SANDSTONE, 31, HARL, 0], &[2, 25, 22, 2]),
            // Brick Gothic under copper, slate, red and black tile.
            NorthSea => (&[4, KLINKER, 5, 4, 30, 6], &[21, 2, 0, 12, 26, 21]),
            Florida => (&[13, 0, 1, 15], &[16, 18, 15, 0]),
            Malta => (&[HONEY, 31, HONEY], &[0, 22, 16]),
            Aegean => (&[13, 18, 0, 31, 30], &[0, 16, 0, 1]),
            NorthAfrica => (&[13, 0, 31, 15], &[0, 16, 15]),
            Nile => (&[15, 1, 13], &[14, 15]),
            // White board and coral-stone churches under grey shingle, green and red iron.
            Hawaii => (&[13, 0, 30, 13, 1], &[18, 5, 4, 7, 20]),
            Micronesia => (&[13, 0, 1, 31], &[4, 7, 0, 15]),
            Atoll => (&[13, 0, 1, 18], &[7, 4, 29, 15, 6]),
        },
        Kind::Tank => (&[13, 13, 2, 11, 15], &[15, 10, 7, 29]),
        // An American row house: brick (painted white or sided now and then), its roof flat or slate.
        Kind::Terrace if region == American => {
            (&[4, 4, 5, 4, 5, 6, 13, 0, 21, KLINKER], &[2, 18, 20, 3])
        }
        _ => match region {
            // Painted timber: white, Falu red, yellow and ochre, grey; black and dark grey tile, red tile now and then.
            Nordic => (
                &[13, 13, 13, 24, 24, 17, 10, 1, 21, 23, 12, 13, 24, 0, 19],
                &[26, 12, 26, 2, 0, 23, 12, 26, 1, 3, 27, 26],
            ),
            Iceland => (
                &[13, 13, 1, 2, 7, 8, 10, 9, 0, 3, 24, 24],
                &[4, 4, 5, 6, 3, 12, 7, 12, 2],
            ),
            Temperate => (
                &[4, 5, 4, 1, 0, 6, 5, 0, 30, 31],
                &[2, 2, 25, 26, 1, 23, 13, 0, 2],
            ),
            Mediterranean => (
                &[18, 0, 1, 17, 19, 20, 16, 10, 9, 31, 13, 18],
                &[0, 16, 17, 1, 0, 16, 28, 23, 0],
            ),
            American => (
                &[21, 22, 13, 23, 4, 1, 0, 8, 7, 5, 22],
                &[18, 19, 20, 18, 3, 12, 2, 19],
            ),
            Tropical => (
                &[0, 13, 1, 7, 8, 9, 10, 15, 2, 12],
                &[7, 8, 7, 8, 7, 29, 4, 8, 6, 5, 0, 7],
            ),
            Arid => (&[15, 1, 0, 13, 10, 15, 31, 19], &[0, 1, 14, 15, 0, 9]),
            // Light stucco under grey and brown asphalt shingles; one roof in ten of concrete tile.
            West => (
                &[0, 1, 13, 15, 18, 19, 20, 22, 31, 21, 1, 0, 9, 7],
                &[18, 19, 20, 18, 19, 3, 20, 18, 28, 19],
            ),
            // Light walls (a dark timber house now and then) under grey kawara and dark glazed tile, or grey, blue
            // and green sheet metal.
            EastAsia => (
                &[0, 1, 2, 13, 21, 22, 27, 31, 15, 0, 13, 12],
                &[KAWARA, KAWARA, 26, 12, 25, KAWARA, 22, 7, 6, 5, 26, 27, 6],
            ),
            // Taiwan's townhouses in small tiles, darker than Japan's walls: greys, beige, tan, salmon and brown
            // (brick-red tile now and then), a white or cream one; the few pitched roofs blue, grey and rusted sheet,
            // red clay tile and kawara.
            Taiwan => (
                &[
                    TILE_GREY, 3, 30, TILE_DUN, TILE_TAN, 16, 22, 15, TILE_BROWN, 5, TILE_DARK,
                    TILE_MAUVE, 3, TILE_GREY, 4, TILE_ROSE, 2, 30, TILE_OLIVE, 13,
                ],
                &[6, 7, 8, 29, 1, 0, 6, 27, KAWARA, 7, 15, 8],
            ),
            // Red and brown brick (yellow stock, white and cream render, stone now and then) under slate, two roofs in
            // three, and brown, red-brown and grey concrete tile.
            British => (
                &[4, 4, 5, 4, 6, 5, 0, 1, 4, 5, 31, 4, KLINKER, 0],
                &[2, 25, 2, 23, 13, 26, 2, 1, 25, 23, 2, 26],
            ),
            // Grey harl, white and cream render, grey and red sandstone, under slate.
            Celtic => (
                &[
                    HARL,
                    0,
                    30,
                    HARL,
                    1,
                    13,
                    30,
                    RED_SANDSTONE,
                    0,
                    HARL,
                    17,
                    31,
                    HARL,
                    2,
                ],
                &[2, 25, 2, 26, 25, 2, 3, 26, 2],
            ),
            // Red, purple-brown and yellow brick (white now and then) under orange and red pantiles, black glazed tile,
            // thatch.
            NorthSea => (
                &[4, KLINKER, 5, 4, 6, 4, 0, KLINKER, 5, 13, 4, 1, 6, 4],
                &[16, 0, 26, 12, 16, 23, 1, 26, THATCH, 0, 2, 16, 12, 23],
            ),
            // White and pastel stucco under grey and brown shingle, barrel tile, white roofs and metal.
            Florida => (
                &[0, 13, 1, 20, 9, 7, 8, 17, 15, 19, 0, 13, 1, 22],
                &[18, 19, 15, 16, 17, 28, 20, 7, 18, 29, 15, 19, 16],
            ),
            // Honey limestone, cream and pale ochre; the few pitched roofs terracotta.
            Malta => (
                &[HONEY, HONEY, 31, HONEY, 1, 19, HONEY, 31, 15, 0, HONEY, 17],
                &[0, 16, 17],
            ),
            // Whitewash, white and cream, pale ochre, peach and warm yellow, a pastel now and then, under flat roofs
            // or terracotta.
            Aegean => (
                &[18, 13, 0, 18, 1, 19, 20, 17, 13, 31, 18, 0, 7],
                &[0, 16, 17, 1, 0, 23, 16],
            ),
            // White, whitewash, cream, sand and ochre, a Marrakech pink now and then; a colonial villa's terracotta
            // or a green-glazed roof.
            NorthAfrica => (
                &[0, 18, 13, 1, 15, 19, 10, 20, 15, 18, 31, 9, 1, 16],
                &[0, 16, 1, 5],
            ),
            // Red brick between concrete frames, beige, sand and ochre render.
            Nile => (&[4, 5, 15, 1, 19, 2, 3, 10, 4, 15, 0, 20], &[14, 0]),
            // Painted boards and siding (white, pale green, plantation yellow, tan, grey) and stucco under grey, dark
            // and brown shingle, galvanised, green-grey, green and red iron, a white or concrete-tile roof now and then.
            Hawaii => (
                &[13, 13, 0, 1, 8, 17, 22, 21, 19, 7, 15, 20, 12, 23, 13, 22],
                &[18, 18, 20, 19, 7, 29, 5, 18, 27, 4, 20, 15, 28, 19],
            ),
            // Concrete painted white, cream and pastels (pink, blue, green, peach), under galvanised, red, green and
            // blue iron on the few pitched.
            Micronesia => (
                &[0, 13, 1, 2, 7, 8, 9, 20, 15, 19, 17, 18, 3, 13, 0],
                &[7, 29, 8, 4, 5, 6, 7, 15, 8, 29],
            ),
            // Concrete block, plywood and sheet, painted now and then, under galvanised and rusted iron, pandanus
            // thatch on one in six.
            Atoll => (
                &[2, 0, 1, 11, 7, 8, 9, 12, 15, 30, 13, 20, 3, 27],
                &[7, 8, 7, 29, THATCH, 8, 4, 6, 7, THATCH, 5, 8, 29],
            ),
        },
    };
    let sloped_or_metal =
        pitched || matches!(kind, Kind::Industrial | Kind::Warehouse | Kind::Tank);
    let roof = if sloped_or_metal {
        pick(roofs, b)
    } else {
        pick(flat_roofs(region), b)
    };
    (pick(walls, a), roof)
}

/// A past period's flat roofs: tar, gravel, bitumen felt and concrete (no membranes).
const PAST_FLAT_ROOFS: &[u8] = &[11, 9, 11, 3, 14, 9, 12, 11];
/// A warm, dry country's then: concrete, limewash and light gravel.
const PAST_PALE_FLAT_ROOFS: &[u8] = &[14, 15, 14, 9, 14, 15, 9];
/// The Pacific's few flat roofs then: concrete, tar and gravel.
const PAST_PACIFIC_FLAT_ROOFS: &[u8] = &[14, 9, 11, 14, 9, 3];
const PAST_MALTA_FLAT_ROOFS: &[u8] = &[SCREED, 14, SCREED, SCREED, 15, SCREED];
const PAST_AFRICA_FLAT_ROOFS: &[u8] = &[15, SCREED, 14, 15, SCREED, 9];
const PAST_DUSTY_FLAT_ROOFS: &[u8] = &[14, 9, 28, 14, 9, 14];

/// A region's flat roofs in a past period.
fn past_flat_roofs(region: Region) -> &'static [u8] {
    use Region::*;
    match region {
        Mediterranean | Aegean | Arid => PAST_PALE_FLAT_ROOFS,
        Hawaii | Micronesia | Atoll | Tropical => PAST_PACIFIC_FLAT_ROOFS,
        Malta => PAST_MALTA_FLAT_ROOFS,
        NorthAfrica => PAST_AFRICA_FLAT_ROOFS,
        Nile => PAST_DUSTY_FLAT_ROOFS,
        _ => PAST_FLAT_ROOFS,
    }
}

/// The walls' and roof's entries as a building stood in a past period (native/docs/terrain/period.md, "Period
/// styling"): more brick, stone, timber and render and fewer pastels; flat roofs in tar, gravel, felt or concrete; no
/// glass, concrete panels, coloured sheet or concrete tile. By region: the American West's clay tile, wood and
/// composition shingle; America's houses in painted clapboard, brick in the East; Britain, the North Sea and Europe
/// sootier (the shader darkens them too); Japan's charred and dark-stained boards and white plaster under grey kawara,
/// Western masonry downtown; the tropics' timber under corrugated iron and sago-leaf thatch; Hawaii's and Micronesia's
/// plantation timber and iron; the Mediterranean, Malta, the Aegean and North Africa much as now.
fn past_colours(region: Region, kind: Kind, pitched: bool, a: f32, b: f32) -> (u8, u8) {
    use Region::*;
    let (walls, roofs): (&[u8], &[u8]) = match kind {
        Kind::Industrial | Kind::Warehouse => match region {
            // Sheds of sheet and timber under galvanised and rusted iron.
            Tropical | Micronesia | Atoll | Hawaii => {
                (&[11, 12, 11, 13, 11, 12], &[7, 8, 7, 4, 29, 8])
            }
            Arid | NorthAfrica | Nile | Malta => (&[15, 31, 13, 11, 15, 1], &[14, 7, 29, 9]),
            // Brick mills and warehouses, a shed of sheet now and then, under slate and iron.
            British | Celtic | NorthSea => (
                &[4, 5, 11, 4, KLINKER, 5, 11, 4, 30],
                &[7, 2, 3, 2, 8, 7, 11, 2],
            ),
            _ => (
                &[11, 4, 5, 11, 12, 4, 30, 3, 11],
                &[7, 3, 7, 8, 11, 2, 29, 7],
            ),
        },
        // Masonry offices: limestone, stone, brick and render.
        Kind::Office => (&[31, 30, 4, 5, 1, 31, 6, 30, 0, 4], past_flat_roofs(region)),
        Kind::Block | Kind::Tower => match region {
            Temperate => (&[1, 4, 5, 30, 31, 4, 5, 1, 0, 30], &[2, 25, 13, 23, 2]),
            Mediterranean => (&[18, 0, 1, 17, 19, 20, 31, 16, 30], &[0, 16, 17, 1]),
            Nordic => (&[1, 17, 10, 0, 19, 9, 4, 20, 5, 1], &[12, 21, 12, 2, 4, 0]),
            // Reykjavík's rendered concrete, white and grey.
            Iceland => (&[0, 1, 30, 1, 0, 13, 2], &[4, 12, 2, 5]),
            American => (&[4, 5, 4, 30, 31, 1, 6, 5, RED_SANDSTONE], &[18, 20, 3]),
            Tropical => (&[0, 1, 13, 15, 31, 0], &[7, 8, 4]),
            Arid => (&[15, 1, 0, 31, 15, 19], &[14, 15, 9]),
            // Downtown Los Angeles' brick, terracotta and limestone.
            West => (&[0, 1, 15, 31, 19, 20, 18, 4, 5, 30], &[0, 18, 20, 16]),
            // Western masonry downtown: brick, stone and render.
            EastAsia | Taiwan => (&[4, 5, 30, 31, 1, 0, 4, 31], &[KAWARA, 26, 2, IBUSHI]),
            British => (&[4, 5, HARL, 1, 6, 4, 30, 5, 4, 31], &[2, 25, 2, 23]),
            Celtic => (
                &[HARL, 30, RED_SANDSTONE, 0, 31, HARL, 1, 30, RED_SANDSTONE],
                &[2, 25, 2, 2],
            ),
            NorthSea => (
                &[4, KLINKER, 5, 6, 4, 0, 1, 5, KLINKER],
                &[0, 16, 26, 12, 23],
            ),
            Florida => (&[0, 1, 15, 19, 0, 20, 31, 1], &[16, 0, 18, 17]),
            Malta => (
                &[HONEY, 31, 1, HONEY, 19, HONEY, 15, 13, HONEY, 0],
                &[0, 16],
            ),
            Aegean => (&[0, 18, 1, 19, 31, 15, 20, 1, 0, 18], &[0, 16]),
            NorthAfrica => (&[0, 18, 1, 19, 15, 31, 20, 0, 18], &[0, 16]),
            Nile => (&[15, 1, 19, 10, 4, 5, 1, 0, 20, 15, 31], &[14]),
            Hawaii => (&[0, 13, 1, 15, 31, 1, 0, 22, 19, 4], &[7, 29, 18, 0]),
            Micronesia => (&[0, 1, 15, 31, 13, 19, 0], &[7, 29, 4]),
            Atoll => (&[0, 1, 15, 13, 12], &[7, 8, 29, 4]),
        },
        // Churches stood then as now; a Japanese temple's boards charred or dark.
        Kind::Church if matches!(region, EastAsia | Taiwan) => {
            (&[12, 13, CHARRED, 12, 30], &[KAWARA, 26, 12, 21, KAWARA])
        }
        Kind::Church => {
            let (wall, roof) = colours(region, kind, Period::Today, pitched, a, b);
            let roof = if pitched {
                roof
            } else {
                pick(past_flat_roofs(region), b)
            };
            return (wall, roof);
        }
        Kind::Tank => (&[13, 13, 2, 11, 15], &[15, 7, 29, 7]),
        // An American row house: brick and brownstone, its roof flat or slate.
        Kind::Terrace if region == American => (
            &[4, 4, 5, 4, 5, 6, RED_SANDSTONE, 4, 5, KLINKER, 30],
            &[2, 18, 20, 3],
        ),
        _ => match region {
            // Painted timber, as now.
            Nordic => (
                &[13, 13, 13, 24, 24, 17, 10, 1, 21, 23, 12, 13, 24, 0, 19],
                &[26, 12, 26, 2, 0, 23, 12, 26, 1, 1, 27, 26],
            ),
            // Corrugated iron and boards painted white, cream and red under red, green and grey iron and tar.
            Iceland => (
                &[13, 13, 1, 2, 0, 3, 24, 13, 30, 10, 24, 1],
                &[4, 4, 5, 7, 12, 12, 2, 4, 6],
            ),
            Temperate => (
                &[4, 5, 5, 4, 30, 1, 5, 30, 31, 4, 0, 6],
                &[2, 2, 25, 1, 23, 13, 0, 2, 12],
            ),
            Mediterranean => (
                &[18, 0, 1, 17, 19, 20, 16, 10, 31, 18, 15, 30],
                &[0, 16, 17, 1, 0, 16, 23, 0, 1],
            ),
            // Clapboard painted white, cream, yellow and grey, brick in the East's towns.
            American => (
                &[13, 13, 1, 4, 21, 13, 17, 5, 4, 0, 22, 13],
                &[18, 19, 20, 2, 19, 18, 12, 3],
            ),
            // Timber houses under corrugated iron and sago-leaf thatch.
            Tropical => (
                &[12, 13, 1, 15, 12, 0, 22, 13, 12, 19],
                &[THATCH, 7, 8, THATCH, 7, 8, 29, THATCH, 4, 8],
            ),
            Arid => (&[15, 1, 0, 18, 10, 15, 31, 19], &[0, 1, 14, 15, 0, 9]),
            // Stucco and clapboard bungalows under clay tile, wood shingle and composition shingle.
            West => (
                &[0, 1, 13, 15, 18, 19, 20, 31, 1, 0, 22, 13, 17],
                &[0, 19, 18, 16, 19, 20, 0, 18, 1, 19, 17, 18],
            ),
            // Charred and dark-stained boards and white shikkui plaster under kawara, smoked and weathered grey, some
            // glazed black; a poorer house's rusted tin.
            EastAsia | Taiwan => (
                &[CHARRED, 12, CHARRED, 13, 12, 15, CHARRED, 0, 12, 19, 13],
                &[
                    IBUSHI, KAWARA, IBUSHI, GLAZED, IBUSHI, 26, KAWARA, IBUSHI, 8, IBUSHI, GLAZED,
                    12, KAWARA,
                ],
            ),
            // Sooty brick, slate and darker render (harl).
            British => (
                &[4, 5, 5, 4, 6, 5, HARL, 4, 5, 30, KLINKER, 4, 1],
                &[2, 25, 2, 2, 23, 13, 2, 1, 25, 2, 23],
            ),
            // Harl, sandstone and whitewash under slate, an Irish cottage's thatch now and then.
            Celtic => (
                &[
                    HARL,
                    0,
                    30,
                    HARL,
                    1,
                    30,
                    RED_SANDSTONE,
                    HARL,
                    31,
                    HARL,
                    30,
                    18,
                ],
                &[2, 25, 2, 2, 25, 2, THATCH, 2, 12],
            ),
            NorthSea => (
                &[4, KLINKER, 5, 4, 6, 4, KLINKER, 5, 4, 1, 6, 5],
                &[16, 0, 26, 12, 16, 23, 1, 26, THATCH, 0, 2, 16, 12, 23],
            ),
            // Stucco and frame houses, few pastels, under shingle, barrel tile and iron.
            Florida => (
                &[0, 1, 13, 15, 19, 17, 0, 1, 20, 18, 22],
                &[18, 19, 16, 0, 17, 7, 20, 29, 18, 16, 7],
            ),
            Malta => (
                &[HONEY, HONEY, 31, HONEY, 1, 19, HONEY, 31, 15, 0, HONEY, 17],
                &[0, 16, 17],
            ),
            Aegean => (
                &[18, 18, 0, 18, 1, 19, 20, 17, 31, 18, 0, 15],
                &[0, 16, 17, 1, 0, 23, 16],
            ),
            NorthAfrica => (
                &[0, 18, 18, 1, 15, 19, 10, 20, 15, 18, 31, 1, 16],
                &[0, 16, 1, 5],
            ),
            // Mud brick, stone and render, brick now and then.
            Nile => (&[15, 1, 19, 10, 15, 4, 5, 0, 20, 15, 31, 19], &[14, 0]),
            // Plantation timber: white, cream, pale green and yellow boards, redwood, under iron and shingle.
            Hawaii => (
                &[13, 13, 1, 8, 17, 22, 19, 12, 13, 1, 12, 15],
                &[7, 29, 5, 4, 19, 7, 8, 18, 5, 7, 29],
            ),
            // Timber under galvanised, rusted and red iron, thatch now and then.
            Micronesia => (
                &[13, 12, 1, 0, 15, 13, 12, 22, 19, 1],
                &[7, 8, 29, 4, 7, 8, THATCH, 7, 5],
            ),
            // Pandanus thatch over timber and plaited walls, iron now and then.
            Atoll => (
                &[12, 13, 15, 12, 11, 0, 22, 12, 15],
                &[THATCH, THATCH, 7, 8, THATCH, 8, 7, THATCH, 29],
            ),
        },
    };
    let sloped_or_metal =
        pitched || matches!(kind, Kind::Industrial | Kind::Warehouse | Kind::Tank);
    let roof = if sloped_or_metal {
        pick(roofs, b)
    } else {
        pick(past_flat_roofs(region), b)
    };
    (pick(walls, a), roof)
}

/// How often a region's houses wear shutters and its blocks balconies (`Building::detail`).
pub(super) fn detail_share(region: Region, kind: Kind) -> f32 {
    use Region::*;
    match kind {
        Kind::House | Kind::Terrace => match region {
            Mediterranean => 0.6,
            Temperate => 0.2,
            American => 0.35,
            Tropical => 0.15,
            Arid => 0.1,
            Nordic | Iceland => 0.0,
            West => 0.15,
            EastAsia => 0.05,
            Taiwan => 0.05,
            // Shutters are rare in Britain, Holland and Denmark.
            British | Celtic => 0.03,
            NorthSea => 0.08,
            Florida => 0.2,
            // Malta's balconies and shutters, Greece's and the Maghreb's shutters.
            Malta => 0.6,
            Aegean => 0.5,
            NorthAfrica => 0.4,
            Nile => 0.1,
            // Hawaii's jalousies and lanais.
            Hawaii => 0.3,
            Micronesia => 0.15,
            Atoll => 0.05,
        },
        Kind::Block | Kind::Tower => match region {
            Mediterranean => 0.7,
            Temperate => 0.4,
            Nordic | Iceland => 0.45,
            American => 0.25,
            Tropical => 0.4,
            Arid => 0.5,
            West => 0.35,
            // A Japanese block's balconies, floor over floor.
            EastAsia => 0.85,
            // A walk-up's or a tower's balconies, often caged.
            Taiwan => 0.6,
            British => 0.3,
            Celtic => 0.2,
            NorthSea => 0.45,
            // A condominium's balconies.
            Florida => 0.6,
            Malta => 0.8,
            // A polykatoikia's balconies, floor over floor.
            Aegean => 0.9,
            NorthAfrica => 0.6,
            Nile => 0.6,
            // A Waikiki condominium's lanais.
            Hawaii => 0.85,
            Micronesia => 0.5,
            Atoll => 0.2,
        },
        // Offices: a curtain wall rather than ribbons of windows.
        Kind::Office => 0.5,
        _ => 0.0,
    }
}

/// How a region roofs a house, shed or terrace (and the low blocks it hips): the share left flat, the share of low
/// blocks under a hipped tile roof, the pitch (degrees: the least and the spread above it) and the share of pitched
/// roofs hipped rather than gabled.
pub(super) struct RoofRules {
    pub flat: f32,
    pub block: f32,
    pub pitch: [f32; 2],
    pub hipped: f32,
}

/// In a past `period` ([`past_roof_rules`]): Micronesia's and Hawaii's houses under iron, not concrete, more of
/// Europe's low blocks under tile and slate, and many of the Pacific's and Japan's under iron and kawara.
pub(super) fn roof_rules(region: Region, kind: Kind, period: Period) -> RoofRules {
    let region = region.in_period(period);
    let rules = todays_roof_rules(region, kind);
    if period.is_past() {
        past_roof_rules(region, rules)
    } else {
        rules
    }
}

fn past_roof_rules(region: Region, rules: RoofRules) -> RoofRules {
    use Region::*;
    let flat = match region {
        // Guam's and Saipan's concrete typhoon houses came after the war.
        Micronesia => 0.15,
        Hawaii => 0.05,
        _ => rules.flat,
    };
    let block = match region {
        Temperate => 0.45,
        British => 0.4,
        Celtic | NorthSea => 0.5,
        Nordic => 0.55,
        Mediterranean => 0.5,
        // A colonial store's or bungalow's hipped iron; Japan's two- and three-storey shops and inns under kawara.
        Tropical => 0.65,
        Atoll => 0.5,
        EastAsia | Taiwan => 0.45,
        Micronesia => 0.4,
        Hawaii => 0.35,
        _ => rules.block,
    };
    RoofRules {
        flat,
        block,
        ..rules
    }
}

fn todays_roof_rules(region: Region, kind: Kind) -> RoofRules {
    use Region::*;
    let flat = match (region, kind) {
        // Four in five of a dry country's houses, one in five of the American West's (gravel and tar).
        (Arid, _) => 0.8,
        (West, _) => 0.2,
        // The old cities' brick rows (Boston, New York, Philadelphia, Baltimore, Washington, Norfolk).
        (American, Kind::Terrace) => 0.65,
        (Florida, _) => 0.15,
        // Malta's and Egypt's roofs flat, North Africa's nearly all, Greece's half (the islands' and Athens').
        (Malta, _) => 0.95,
        (Nile, _) => 0.97,
        (NorthAfrica, _) => 0.88,
        (Aegean, _) => 0.55,
        // Guam's and Saipan's concrete roofs; a few on the atolls and in Hawaii.
        (Micronesia, _) => 0.65,
        (Hawaii, _) => 0.1,
        (Atoll, _) => 0.1,
        // Taiwan's concrete townhouses and walk-ups nearly all flat; sheet over a few.
        (Taiwan, _) => 0.95,
        _ => 0.0,
    };
    let block = match region {
        Mediterranean => 0.35,
        Temperate => 0.25,
        // Bergen's, Oslo's and Stockholm's blocks under black tile, copper and red; Reykjavik's mostly flat.
        Nordic => 0.4,
        Iceland => 0.15,
        American | West | Florida => 0.1,
        // Mansion blocks and tenements under slate; Dutch and German blocks under tile.
        British => 0.2,
        Celtic => 0.3,
        NorthSea => 0.35,
        Aegean | Hawaii => 0.05,
        NorthAfrica => 0.03,
        Tropical | Arid | EastAsia | Taiwan | Malta | Nile | Micronesia | Atoll => 0.0,
    };
    let pitch = match (region, kind) {
        (_, Kind::Block) => [18.0, 8.0],
        (_, Kind::Shed) => [14.0, 10.0],
        (Tropical, _) => [18.0, 10.0],
        (Arid, _) => [16.0, 8.0],
        (Mediterranean, _) => [18.0, 9.0],
        (American, _) => [22.0, 14.0],
        // A ranch house's low 4:12 to 6:12; Florida's hipped roofs alike.
        (West, _) => [17.0, 10.0],
        (Florida, _) => [18.0, 8.0],
        // Kawara at about 4 sun in 10, sheet metal lower.
        (EastAsia | Taiwan, _) => [16.0, 12.0],
        // A Victorian terrace's 30-44 degrees, Scotland's steeper slate, Holland's and Denmark's steep tile.
        (British, _) => [30.0, 14.0],
        (Celtic, _) => [35.0, 12.0],
        (NorthSea, _) => [40.0, 14.0],
        (Malta | NorthAfrica, _) => [18.0, 10.0],
        (Aegean, _) => [20.0, 8.0],
        (Nile, _) => [16.0, 8.0],
        // Hawaii's hipped and plantation roofs; low tin in Micronesia, an atoll's tin and steeper thatch.
        (Hawaii, _) => [20.0, 12.0],
        (Micronesia, _) => [14.0, 10.0],
        (Atoll, _) => [16.0, 14.0],
        (Nordic | Iceland | Temperate, _) => [26.0, 16.0],
    };
    let hipped = match region {
        Nordic | Iceland => 0.15,
        Temperate => 0.35,
        Mediterranean => 0.45,
        American => 0.35,
        Tropical => 0.6,
        Arid => 0.5,
        West => 0.55,
        EastAsia => 0.5,
        Taiwan => 0.35,
        British => 0.3,
        Celtic => 0.15,
        NorthSea => 0.25,
        Florida => 0.7,
        Malta | NorthAfrica => 0.4,
        Aegean => 0.45,
        Nile => 0.5,
        Hawaii => 0.7,
        Micronesia => 0.4,
        Atoll => 0.35,
    };
    RoofRules {
        flat,
        block,
        pitch,
        hipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buildings::region::REGIONS;

    const KINDS: [Kind; 14] = [
        Kind::Shed,
        Kind::House,
        Kind::Terrace,
        Kind::Block,
        Kind::Tower,
        Kind::Industrial,
        Kind::Church,
        Kind::Office,
        Kind::Warehouse,
        Kind::Tank,
        Kind::Lighthouse,
        Kind::Chimney,
        Kind::Crane,
        Kind::Deck,
    ];

    /// FNV-1a over every choice the palettes make today, on a grid of hashes.
    fn fingerprint() -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut eat = |b: u8| {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        // Every region's as it was before Taiwan's own (2026-10-29).
        for region in REGIONS.into_iter().filter(|r| *r != Region::Taiwan) {
            for kind in KINDS {
                for pitched in [false, true] {
                    for i in 0..37 {
                        for j in 0..29 {
                            let (a, b) = (i as f32 / 36.0, j as f32 / 28.0);
                            let (wall, roof) = colours(region, kind, Period::Today, pitched, a, b);
                            eat(wall);
                            eat(roof);
                        }
                    }
                }
                for f in detail_share(region, kind).to_le_bytes() {
                    eat(f);
                }
                let r = roof_rules(region, kind, Period::Today);
                for v in [r.flat, r.block, r.pitch[0], r.pitch[1], r.hipped] {
                    for f in v.to_le_bytes() {
                        eat(f);
                    }
                }
            }
        }
        h
    }

    /// Each (wall, roof) a region's `kind` takes in `period`, on a grid of hashes, pitched or not.
    fn choices(region: Region, kind: Kind, period: Period, pitched: bool) -> Vec<(u8, u8)> {
        (0..400)
            .map(|k| {
                let (a, b) = ((k % 20) as f32 / 19.0, (k / 20) as f32 / 19.0);
                colours(region, kind, period, pitched, a, b)
            })
            .collect()
    }

    #[test]
    fn a_past_periods_palettes_hold_no_modern_finish() {
        let past = Period::Year(1942);
        let share = |v: &[(u8, u8)], f: &dyn Fn(u8, u8) -> bool| {
            v.iter().filter(|(w, r)| f(*w, *r)).count() as f32 / v.len() as f32
        };
        let mut pastel = [0.0f32; 2];
        for region in REGIONS {
            for kind in KINDS {
                for pitched in [false, true] {
                    let then = choices(region, kind, past, pitched);
                    // No membranes, glass or glass-like walls.
                    assert_eq!(
                        share(&then, &|_, r| r == 10 || r == 24),
                        0.0,
                        "{region:?} {kind:?}"
                    );
                    assert_eq!(
                        share(&then, &|w, _| matches!(w, 14 | 28 | 29 | 25 | 26 | 27)),
                        0.0,
                        "{region:?} {kind:?}"
                    );
                    let now = choices(region, kind, Period::Today, pitched);
                    if matches!(kind, Kind::House | Kind::Block) {
                        pastel[0] += share(&now, &|w, _| matches!(w, 7..=9));
                        pastel[1] += share(&then, &|w, _| matches!(w, 7..=9));
                    }
                }
            }
        }
        // Fewer pastels.
        assert!(pastel[1] < 0.4 * pastel[0], "{pastel:?}");
        let house = |region| choices(region, Kind::House, past, true);
        // The American West's roofs clay tile and shingle, no concrete tile.
        let west = house(Region::West);
        assert_eq!(share(&west, &|_, r| r == 28), 0.0);
        assert!(share(&west, &|_, r| matches!(r, 0 | 1 | 16 | 17)) > 0.25);
        assert!(share(&west, &|_, r| matches!(r, 18..=20)) > 0.45);
        // Japan's houses dark boards and white plaster under kawara, no blue or green sheet.
        let japan = house(Region::EastAsia);
        assert!(share(&japan, &|w, _| matches!(w, CHARRED | 12)) > 0.4);
        assert!(share(&japan, &|_, r| matches!(r, KAWARA | IBUSHI | GLAZED)) > 0.65);
        // Not one grey: smoked, weathered and glazed kawara, a tin roof now and then.
        for kawara in [KAWARA, IBUSHI, GLAZED] {
            assert!(share(&japan, &|_, r| r == kawara) > 0.1, "{kawara}");
        }
        assert!(share(&japan, &|_, r| matches!(r, 4 | 8)) > 0.05);
        assert_eq!(share(&japan, &|_, r| matches!(r, 5 | 6 | 25 | 27)), 0.0);
        // The Solomons' timber under iron and sago-leaf thatch.
        let tropics = house(Region::Tropical);
        assert!(share(&tropics, &|_, r| r == THATCH) > 0.2);
        assert!(share(&tropics, &|_, r| matches!(r, 7 | 8 | 29 | 4)) > 0.5);
        // Britain's brick and slate, darker than today's.
        let britain = house(Region::British);
        assert!(share(&britain, &|w, _| matches!(w, 4 | 5 | 6 | KLINKER)) > 0.6);
        assert!(share(&britain, &|w, _| w == 0) == 0.0);
        // Malta's as now.
        assert_eq!(
            house(Region::Malta),
            choices(Region::Malta, Kind::House, Period::Today, true)
        );
        // Micronesia's houses under iron, not concrete.
        assert!(roof_rules(Region::Micronesia, Kind::House, past).flat < 0.2);
        assert_eq!(
            roof_rules(Region::Micronesia, Kind::House, Period::Today).flat,
            0.65
        );
    }

    #[test]
    fn taiwans_palettes_are_darker_tiles_under_flat_concrete() {
        let share = |v: &[(u8, u8)], f: &dyn Fn(u8, u8) -> bool| {
            v.iter().filter(|(w, r)| f(*w, *r)).count() as f32 / v.len() as f32
        };
        let tiles = [
            2, 3, 15, 16, 22, 30, 4, 5, TILE_GREY, TILE_DARK, TILE_TAN, TILE_DUN, TILE_OLIVE,
            TILE_BROWN, TILE_MAUVE, TILE_ROSE,
        ];
        // `buildings.wgsl`'s `WALLS` (0-31) and the colour cube.
        const WALLS: [[u8; 3]; 32] = [
            [206, 202, 190],
            [200, 186, 156],
            [168, 166, 158],
            [142, 141, 136],
            [146, 78, 60],
            [116, 80, 64],
            [190, 164, 116],
            [148, 166, 180],
            [154, 174, 138],
            [192, 158, 148],
            [204, 168, 96],
            [160, 165, 168],
            [96, 72, 56],
            [216, 216, 210],
            [88, 98, 108],
            [190, 174, 142],
            [200, 140, 104],
            [214, 184, 112],
            [222, 218, 206],
            [210, 182, 136],
            [214, 180, 150],
            [178, 180, 176],
            [196, 184, 160],
            [128, 142, 152],
            [128, 44, 36],
            [74, 104, 140],
            [88, 120, 96],
            [196, 190, 170],
            [70, 92, 112],
            [78, 104, 100],
            [150, 144, 132],
            [196, 188, 166],
        ];
        let luma = |w: u8| -> f32 {
            let rgb = if w < CUBE_FIRST {
                WALLS[usize::from(w)]
            } else if w < EXTRA_FIRST {
                let i = w - CUBE_FIRST;
                [i / 36 * 51, i / 6 % 6 * 51, i % 6 * 51]
            } else {
                [200, 200, 200]
            };
            0.2126 * f32::from(rgb[0]) + 0.7152 * f32::from(rgb[1]) + 0.0722 * f32::from(rgb[2])
        };
        let mean = |v: &[(u8, u8)]| v.iter().map(|(w, _)| luma(*w)).sum::<f32>() / v.len() as f32;
        for kind in [Kind::House, Kind::Terrace, Kind::Block, Kind::Tower] {
            let now = choices(Region::Taiwan, kind, Period::Today, false);
            // The tiles' greys, beige, tan, salmon and brown on four in five; spread over many, none a fifth.
            assert!(share(&now, &|w, _| tiles.contains(&w)) > 0.8, "{kind:?}");
            let mut seen = std::collections::HashMap::new();
            for (w, _) in &now {
                *seen.entry(*w).or_insert(0usize) += 1;
            }
            assert!(seen.len() >= 10, "{kind:?} {}", seen.len());
            assert!(
                seen.values().all(|n| *n < now.len() / 5),
                "{kind:?} {seen:?}"
            );
            // Darker than Japan's walls.
            let japan = choices(Region::EastAsia, kind, Period::Today, false);
            assert!(
                mean(&now) + 15.0 < mean(&japan),
                "{kind:?} {} {}",
                mean(&now),
                mean(&japan)
            );
            // Flat roofs mostly grey concrete and green-grey waterproofing, a heat tile's red now and then, no
            // membranes.
            assert!(share(&now, &|_, r| matches!(r, 14 | 27)) > 0.45, "{kind:?}");
            assert!(share(&now, &|_, r| r == 23) < 0.12, "{kind:?}");
            assert_eq!(share(&now, &|_, r| matches!(r, 10 | 24)), 0.0, "{kind:?}");
            assert!(
                roof_rules(Region::Taiwan, kind, Period::Today).flat >= 0.95 || kind != Kind::House
            );
            // Its past is Japan's.
            assert_eq!(
                choices(Region::Taiwan, kind, Period::Year(1942), true),
                choices(Region::EastAsia, kind, Period::Year(1942), true)
            );
        }
        assert_eq!(
            roof_rules(Region::Taiwan, Kind::Block, Period::Today).block,
            0.0
        );
        assert_eq!(
            roof_rules(Region::Taiwan, Kind::House, Period::Year(1942)).flat,
            roof_rules(Region::EastAsia, Kind::House, Period::Year(1942)).flat
        );
    }

    #[test]
    fn todays_palettes_are_as_they_were() {
        assert_eq!(fingerprint(), 9_223_308_204_072_266_718);
    }
}
