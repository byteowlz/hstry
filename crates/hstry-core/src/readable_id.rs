//! Human-readable conversation IDs (`adjective-noun`, e.g. `cold-lamp`).
//!
//! The word lists are shared across byteowlz tools (see the `schemas/wordlist`
//! repo) and embedded at compile time so the binary stays self-contained.
//!
//! IDs are derived deterministically from the conversation UUID so the same
//! conversation resolves to the same base form on any machine. Collisions
//! (birthday bound is ~660 conversations for the ~441k combination space) are
//! resolved by the caller by appending a numeric suffix.

use std::sync::OnceLock;
use uuid::Uuid;

#[derive(serde::Deserialize)]
struct WordLists {
    adjectives: Vec<String>,
    nouns: Vec<String>,
}

static WORDS: OnceLock<WordLists> = OnceLock::new();

fn words() -> &'static WordLists {
    WORDS.get_or_init(|| {
        const TOML: &str = include_str!("../data/word_lists.toml");
        toml::from_str(TOML).expect("bundled word_lists.toml must parse")
    })
}

/// SplitMix64 finaliser — spreads nearby seeds apart so sequential UUIDs map to
/// unrelated words.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn split(u: u128) -> (u64, u64) {
    ((u >> 64) as u64, u as u64)
}

/// Deterministically derive the base `adjective-noun` form for a conversation.
///
/// The caller must resolve collisions against the database (e.g. by appending a
/// suffix via [`suffix_for`]).
pub fn base_for(uuid: Uuid) -> String {
    let w = words();
    let (hi, lo) = split(uuid.as_u128());
    let adj = mix(hi) as usize % w.adjectives.len();
    let noun = mix(lo) as usize % w.nouns.len();
    format!("{}-{}", w.adjectives[adj], w.nouns[noun])
}

/// A deterministic starting suffix (>= 2) for collision resolution, so the same
/// UUID lands on the same final id regardless of ingestion order.
pub fn suffix_for(uuid: Uuid) -> u32 {
    let (hi, lo) = split(uuid.as_u128());
    let v = mix(hi ^ lo.rotate_left(23)) % 998;
    2 + (v as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_is_stable_and_shaped() {
        let u = Uuid::parse_str("94d56814-0000-0000-0000-000000000000").unwrap();
        let a = base_for(u);
        let b = base_for(u);
        assert_eq!(a, b, "deterministic for the same UUID");
        let parts: Vec<&str> = a.split('-').collect();
        assert_eq!(parts.len(), 2, "expected adjective-noun, got {a}");
    }

    #[test]
    fn nearby_uuids_diverge() {
        let u0 = Uuid::parse_str("00000000-0000-0000-0000-000000000000").unwrap();
        let u1 = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        assert_ne!(base_for(u0), base_for(u1));
    }
}
