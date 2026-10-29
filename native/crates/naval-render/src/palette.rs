//! The game's interface palette for the marks the native client draws itself: the ship labels and their hull meters,
//! the gun-aim circles and the torpedo sight. A mirror of the page's tokens (`src/ui/tokens.css`, whose TypeScript
//! mirror is `src/ui/palette.ts`); the test below reads that file and fails when a value here and its token differ.
//!
//! The rule (DESIGN.md, "The Side and State Rule"): colour only for side and state. [`OWN`] blue is ours, [`FOE`] red
//! is theirs and what is lost or refused, [`UNKNOWN`] amber is unconfirmed or a warning, [`READY`] green is only for
//! a thing literally ready (a gun laid, a firing solution), [`FIRE`] orange is fire, and a selection is [`SELECT`]
//! white. Marks drawn straight over the live sea take [`SEA_TEXT`], [`SEA_MUTED`] and [`SEA_LINE`], with
//! [`SEA_SHADOW`] under their text. The world (ships, sea, sky, effects) keeps its own colours.

/// Declares each token as a constant named for it and lists them all in [`TOKENS`], so none escapes the drift test.
macro_rules! tokens {
    ($($(#[$doc:meta])* $name:ident = $token:literal $value:literal;)*) => {
        $($(#[$doc])* pub const $name: &str = $value;)*
        /// Every token mirrored here: its name in `tokens.css` (without `--ui-`) and its value.
        pub const TOKENS: &[(&str, &str)] = &[$(($token, $value)),*];
    };
}

tokens! {
    /// `--ui-shade`: the dark under shadows, scrims and halos (the aim circles' drop shadow, the meter's ground).
    SHADE = "shade" "#05080a";
    /// `--ui-own`: ours. Friendly labels and their meters, our torpedo salvo and the arc it may run in.
    OWN = "own" "#62aef0";
    /// `--ui-foe`: theirs, and what cannot be done. Enemy labels and their meters, a gun that cannot fire, a blocked
    /// torpedo limit.
    FOE = "foe" "#f2695c";
    /// `--ui-unknown`: unconfirmed, and warnings.
    UNKNOWN = "unknown" "#e2b33c";
    /// `--ui-ready`: ready, only. A gun on aim, a torpedo salvo on its firing solution.
    READY = "ready" "#7fc98a";
    /// `--ui-fire`: fire.
    FIRE = "fire" "#ff8a4c";
    /// `--ui-select`: the selection, and the torpedo sight's meeting bar.
    SELECT = "select" "#ffffff";
    /// `--ui-sea-text`: text and marks over the live sea (label names, guns still turning).
    SEA_TEXT = "sea-text" "#f1f2f3";
    /// `--ui-sea-muted`: quieter marks over the sea (a reloading gun, a sunk hull's name, the lead wedge unsolved).
    SEA_MUTED = "sea-muted" "#c5c9cc";
    /// `--ui-sea-line`: a hairline over the sea (the hull meter's edge).
    SEA_LINE = "sea-line" "#ffffff40";
    /// `--ui-sea-shadow`: the text shadow under marks over the sea (not a colour: two CSS shadow layers).
    SEA_SHADOW = "sea-shadow" "0 1px 2px #05080ad9, 0 2px 6px #05080ab3";
    /// `--ui-bevel`: a bevelled control's face, and the steady tone the damage view (I) fills her hull with.
    BEVEL = "bevel" "#3c4d5c";
    /// `--ui-bar`: the deck's ground, which theatre's table fades into toward its horizon.
    BAR = "bar" "#111315";
    /// `--ui-sea`: the chart's sea (theatre's table is the chart tilted: its sea, depth bands, land and grid).
    CHART_SEA = "sea" "#232d35";
    /// `--ui-sea-shoal` (the shallows, at the shore) and `--ui-sea-shelf` (about 50 m): the chart's sea shades from
    /// them to `--ui-sea` (200 m).
    CHART_SEA_SHOAL = "sea-shoal" "#3e5a69";
    CHART_SEA_SHELF = "sea-shelf" "#2c4150";
    /// `--ui-sea-1000`, `--ui-sea-3000`, `--ui-sea-4000`: the chart's sea at least that many metres deep.
    CHART_SEA_1000 = "sea-1000" "#1f2a34";
    CHART_SEA_3000 = "sea-3000" "#19232e";
    CHART_SEA_4000 = "sea-4000" "#141d29";
    /// `--ui-land`, `--ui-land-high` and `--ui-coast`: the chart's land at the shore, toward its heights, and its
    /// coastline.
    CHART_LAND = "land" "#474b45";
    CHART_LAND_HIGH = "land-high" "#6b7066";
    CHART_COAST = "coast" "#73786f";
    /// `--ui-line`: a hairline (the hatching off the chart's sheet).
    LINE = "line" "#ffffff21";
    /// `--ui-graticule`: the chart's grid lines.
    CHART_GRATICULE = "graticule" "#ffffff12";
}

/// A colour token's sRGB bytes and alpha, as CSS reads `#rrggbb` (opaque) and `#rrggbbaa`. A malformed colour panics,
/// at compile time where the call is in a constant.
pub const fn rgba8(color: &str) -> [u8; 4] {
    const fn nibble(digit: u8) -> u8 {
        match digit {
            b'0'..=b'9' => digit - b'0',
            b'a'..=b'f' => digit - b'a' + 10,
            b'A'..=b'F' => digit - b'A' + 10,
            _ => panic!("a colour token is #rrggbb or #rrggbbaa"),
        }
    }
    const fn byte(bytes: &[u8], at: usize) -> u8 {
        nibble(bytes[at]) * 16 + nibble(bytes[at + 1])
    }
    let bytes = color.as_bytes();
    assert!(
        (bytes.len() == 7 || bytes.len() == 9) && bytes[0] == b'#',
        "a colour token is #rrggbb or #rrggbbaa"
    );
    let alpha = if bytes.len() == 9 {
        byte(bytes, 7)
    } else {
        255
    };
    [byte(bytes, 1), byte(bytes, 3), byte(bytes, 5), alpha]
}

/// The layers' colours in a CSS shadow list such as [`SEA_SHADOW`], in order (`#rrggbb` or `#rrggbbaa` each).
pub fn shadow_colors(shadow: &str) -> impl Iterator<Item = [u8; 4]> + '_ {
    shadow
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| part.starts_with('#'))
        .map(rgba8)
}

/// Reading the page's stylesheets as plain text, for the tests that hold this mirror to `tokens.css` and each native
/// mark to the token its page twin's rule names (a test reads the files; these only take their text).
pub mod css {
    /// A stylesheet's `--ui-<name>: <value>;` declarations (`tokens.css`), as (name, value).
    pub fn tokens(css: &str) -> Vec<(&str, &str)> {
        rules(css)
            .flat_map(|(_, declarations)| declarations.split(';'))
            .filter_map(|declaration| {
                let (name, value) = uncommented(declaration)
                    .strip_prefix("--ui-")?
                    .split_once(':')?;
                Some((name.trim(), value.trim()))
            })
            .collect()
    }

    /// What follows the last comment in `text` (a selector list or a declaration), trimmed.
    fn uncommented(text: &str) -> &str {
        text.rfind("*/").map_or(text, |end| &text[end + 2..]).trim()
    }

    /// Each rule's selector list and declarations; a rule inside an at-rule comes without the at-rule.
    fn rules(css: &str) -> impl Iterator<Item = (&str, &str)> {
        css.split('}').filter_map(|chunk| {
            let (selectors, declarations) = chunk.rsplit_once('{')?;
            Some((uncommented(selectors.rsplit('{').next()?), declarations))
        })
    }

    /// The declarations of the first rule whose selector list has `selector` as one of its selectors.
    pub fn rule<'a>(css: &'a str, selector: &str) -> Option<&'a str> {
        rules(css)
            .find(|(selectors, _)| selectors.split(',').any(|s| s.trim() == selector))
            .map(|(_, declarations)| declarations)
    }

    /// `property`'s value among a rule's declarations.
    pub fn property<'a>(declarations: &'a str, property: &str) -> Option<&'a str> {
        declarations.split(';').find_map(|declaration| {
            let (name, value) = uncommented(declaration).split_once(':')?;
            (name.trim() == property).then(|| value.trim())
        })
    }

    /// The token a value takes: `<name>` of its first `var(--ui-<name>)`.
    pub fn var(value: &str) -> Option<&str> {
        let (_, rest) = value.split_once("var(--ui-")?;
        rest.split(')').next()
    }

    /// A token's value among `tokens`.
    pub fn value<'a>(tokens: &[(&str, &'a str)], name: &str) -> Option<&'a str> {
        tokens
            .iter()
            .find(|(token, _)| *token == name)
            .map(|(_, value)| *value)
    }

    /// The token a key of the page's `palette.ts` stands for: its name in camel case (`seaMuted` is `sea-muted`,
    /// `sea4000` is `sea-4000`).
    pub fn palette_key(key: &str) -> String {
        let mut name = String::with_capacity(key.len() + 4);
        let mut previous_digit = false;
        for c in key.chars() {
            let digit = c.is_ascii_digit();
            if c.is_ascii_uppercase() || (digit && !previous_digit && !name.is_empty()) {
                name.push('-');
            }
            name.push(c.to_ascii_lowercase());
            previous_digit = digit;
        }
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_colour_is_its_token_in_tokens_css() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../src/ui/tokens.css");
        let text = std::fs::read_to_string(path).expect("reading src/ui/tokens.css");
        let tokens = css::tokens(&text);
        assert!(tokens.len() > 20, "tokens.css read {} tokens", tokens.len());
        for (name, value) in TOKENS {
            assert_eq!(
                css::value(&tokens, name),
                Some(*value),
                "--ui-{name}: the native palette says {value}"
            );
        }
    }

    #[test]
    fn stylesheet_rules_read_as_css_does() {
        let sheet = "/* a { x } */ .a, .b .c { color:var(--ui-own); background:color-mix(in srgb, var(--ui-shade) 80%, transparent) }
            @container hud (max-width:600px) { .a { color: red; } }";
        let rule = css::rule(sheet, ".a").unwrap();
        assert_eq!(css::property(rule, "color").and_then(css::var), Some("own"));
        assert_eq!(
            css::property(rule, "background").and_then(css::var),
            Some("shade")
        );
        assert_eq!(css::rule(sheet, ".b"), None);
        assert!(css::rule(sheet, ".b .c").is_some());
        assert_eq!(css::palette_key("seaMuted"), "sea-muted");
        assert_eq!(css::palette_key("sea4000"), "sea-4000");
        assert_eq!(css::palette_key("own"), "own");
    }

    #[test]
    fn colours_read_as_css_does() {
        assert_eq!(rgba8(OWN), [0x62, 0xae, 0xf0, 0xff]);
        assert_eq!(rgba8(SEA_LINE), [0xff, 0xff, 0xff, 0x40]);
        assert_eq!(rgba8("#ABCDEF"), [0xab, 0xcd, 0xef, 0xff]);
        let layers: Vec<_> = shadow_colors(SEA_SHADOW).collect();
        assert_eq!(layers, [[0x05, 0x08, 0x0a, 0xd9], [0x05, 0x08, 0x0a, 0xb3]]);
    }

    #[test]
    #[should_panic]
    fn a_malformed_colour_is_refused() {
        rgba8("#12345");
    }
}
