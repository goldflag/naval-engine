//! Visual randomness, local to the effects and seeded by the event that spawned them: the web client's
//! `randomFor` / `seeded` (the web game's `CombatEffects.ts`, `BlastEffects.ts`), a 32-bit LCG. Combat never
//! consumes it, and the same event draws the same puffs on every run.

/// The web's linear congruential stream: `state = state × 1664525 + 1013904223 (mod 2³²)`, returned as
/// `state / 2³²` in [0, 1).
#[derive(Clone, Debug)]
pub struct Lcg {
    state: u32,
}

impl Lcg {
    /// A stream from a seed already reduced to 32 bits.
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    /// A stream from a JavaScript number seed, reduced as `seed >>> 0` does (ToUint32).
    pub fn from_number(seed: f64) -> Self {
        Self::new(to_uint32(seed))
    }

    /// An event's stream, `randomFor(event.sequence * 7919 + (event.shell?.id ?? 0))` of the web game's
    /// `CombatEffects.ts` in JavaScript's number arithmetic: the water an event throws (`naval-water-effects`' splashes).
    pub fn for_event(sequence: u64, shell_id: Option<i64>) -> Self {
        Self::from_number(sequence as f64 * 7919.0 + shell_id.unwrap_or(0) as f64)
    }

    /// The next value in [0, 1).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f64 {
        self.state = self
            .state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        f64::from(self.state) / 4_294_967_296.0
    }
}

/// ECMAScript ToUint32: truncate toward zero, then reduce modulo 2³² (NaN and infinities give 0).
pub fn to_uint32(value: f64) -> u32 {
    if !value.is_finite() {
        return 0;
    }
    let truncated = value.trunc();
    let modulus = 4_294_967_296.0;
    let reduced = truncated.rem_euclid(modulus);
    reduced as u32
}

/// `(n × φ⁻¹ mod 1) × 100`: the pools' per-particle seed, `(++emitted * .61803398875 % 1) * 100`.
pub fn golden_seed(emitted: u64) -> f64 {
    (emitted as f64 * 0.618_033_988_75 % 1.0) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_web_stream() {
        // `randomFor(1)` in the browser: state 1 → 1015568748, then 1586005467.
        let mut random = Lcg::new(1);
        assert_eq!(random.next(), 1_015_568_748.0 / 4_294_967_296.0);
        assert_eq!(random.next(), 1_586_005_467.0 / 4_294_967_296.0);
    }

    #[test]
    fn to_uint32_wraps_like_javascript() {
        assert_eq!(to_uint32(-1.0), u32::MAX);
        assert_eq!(to_uint32(4_294_967_297.5), 1);
        assert_eq!(to_uint32(f64::NAN), 0);
        assert_eq!(to_uint32(7919.0 * 12.0 + 1_000_001.0), 1_095_029);
    }
}
