//! SHA-256 digests, written as 64 lowercase hex digits.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::Sha256;

/// A SHA-256: a square's bytes (its name in the store and its manifest line), a release file (its id), a table.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest(pub [u8; 32]);

impl Digest {
    /// The SHA-256 of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        use sha2::Digest as _;
        Self(Sha256::digest(bytes).into())
    }

    /// 64 lowercase hex digits.
    pub fn hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(64);
        for b in self.0 {
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 15) as usize] as char);
        }
        out
    }

    /// From 64 lowercase hex digits; anything else (upper case included, so one digest has one spelling) is refused.
    pub fn parse(hex: &str) -> Option<Self> {
        let bytes = hex.as_bytes();
        if bytes.len() != 64 {
            return None;
        }
        let digit = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        let mut out = [0u8; 32];
        for (k, pair) in bytes.chunks(2).enumerate() {
            out[k] = digit(pair[0])? << 4 | digit(pair[1])?;
        }
        Some(Self(out))
    }
}

impl std::fmt::Display for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.hex())
    }
}

impl std::fmt::Debug for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Digest({})", self.hex())
    }
}

impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Digest::parse(&text)
            .ok_or_else(|| serde::de::Error::custom("not a SHA-256 in 64 lowercase hex digits"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_has_one_spelling() {
        let d = Digest::of(b"abc");
        assert_eq!(
            d.hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(Digest::parse(&d.hex()), Some(d));
        assert_eq!(Digest::parse(&d.hex().to_uppercase()), None);
        assert_eq!(Digest::parse(&d.hex()[1..]), None);
        assert_eq!(serde_json::to_string(&d).unwrap(), format!("\"{d}\""));
    }
}
