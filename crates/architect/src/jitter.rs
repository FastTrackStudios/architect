//! Deterministic unit floats for jitter — one splitmix64, shared.
//!
//! Both backoff paths need "a reproducible float in `[0, 1)` from a
//! seed": [`schedule::Schedule::jittered`](crate::schedule) spreads
//! retry delays, and [`reconnect::Backoff`](crate::reconnect) spreads
//! reconnect storms. They had a byte-identical splitmix64 each. This is
//! the one copy.
//!
//! splitmix64 rather than `rand`: it is a handful of wrapping
//! multiplies, so it is wasm-clean (no `getrandom`) and reproducible in
//! tests.

/// splitmix64's additive step — the golden-ratio increment.
pub const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// One splitmix64 step: mixes `seed` and returns the mixed state.
///
/// Feed the result back in as the next seed to walk the sequence.
pub const fn splitmix64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(GAMMA);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The mixed state as a float in `[0, 1)`.
///
/// Takes the top 53 bits — exactly `f64`'s mantissa width — so the
/// conversion is lossless and every representable value is reachable.
// The `as f64` pair is the canonical unit-interval conversion. There is
// no total `From`/`TryFrom` for u64 -> f64, and 53 bits is precisely
// what f64 represents exactly, so the "precision loss" the lint warns
// about cannot occur here.
#[allow(clippy::as_conversions, clippy::cast_precision_loss)]
pub fn unit_from_state(state: u64) -> f64 {
    (state >> 11) as f64 / ((1u64 << 53) as f64)
}

/// Deterministic unit float in `[0, 1)` from a seed.
pub fn unit(seed: u64) -> f64 {
    unit_from_state(splitmix64(seed))
}

/// A per-instance RNG seed from wall-clock entropy.
///
/// Two browsers that died together must not retry in lockstep, so the
/// seed has to differ per client AND per instance within a client:
/// sub-second wall-clock nanos give the first, a process-global counter
/// the second. `web_time::SystemTime` works on both native and wasm
/// (`Date.now()` in the browser).
///
/// This is *entropy for spreading*, not for secrecy — never use it where
/// unpredictability matters.
#[cfg(feature = "platform")]
pub fn entropy_seed() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NONCE: AtomicU64 = AtomicU64::new(0);
    let nanos = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0, |d| u64::from(d.subsec_nanos()) ^ d.as_secs());
    nanos ^ NONCE.fetch_add(1, Ordering::Relaxed).rotate_left(32)
}

/// Take the next unit float from a running `state`, advancing it.
///
/// The state walks by [`GAMMA`] and the *output* is mixed — the classic
/// splitmix64 arrangement, and the one `reconnect::Backoff` has always
/// used. Storing the mixed value back as the state instead would be an
/// equally valid generator but a **different** sequence, which is exactly
/// the sort of silent change a jitter refactor must not make.
pub fn next_unit(state: &mut u64) -> f64 {
    let u = unit(*state);
    *state = state.wrapping_add(GAMMA);
    u
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{splitmix64, unit};

    #[test]
    fn unit_stays_in_range_and_is_deterministic() {
        let mut seed = 0u64;
        for _ in 0..10_000 {
            let u = unit(seed);
            assert!((0.0..1.0).contains(&u), "unit out of range: {u}");
            seed = splitmix64(seed);
        }
        assert!((unit(42) - unit(42)).abs() < f64::EPSILON);
        assert!((unit(42) - unit(43)).abs() > f64::EPSILON);
    }
}
