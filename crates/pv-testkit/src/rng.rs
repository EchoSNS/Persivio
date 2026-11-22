//! A tiny deterministic PRNG, so the testkit needs no `rand` dependency and every
//! "random" behavior is reproducible from a seed.

/// `SplitMix64` (Steele, Lea, Flood 2014): fast, decent quality, trivially seedable.
/// **Not** cryptographically secure — for test data and fault injection only.
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Creates a generator from a seed.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform float in `[0, 1)`, from the top 32 bits (exactly representable).
    pub fn next_f64(&mut self) -> f64 {
        let top = u32::try_from(self.next_u64() >> 32).unwrap_or(u32::MAX);
        f64::from(top) / 4_294_967_296.0
    }

    /// Returns `true` with probability `p` (clamped to `[0, 1]`).
    pub fn chance(&mut self, p: f64) -> bool {
        self.next_f64() < p.clamp(0.0, 1.0)
    }

    /// Fills `buf` with random bytes.
    pub fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let bytes = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_per_seed() {
        let a: Vec<u64> = (0..4)
            .map({
                let mut r = SplitMix64::new(1);
                move |_| r.next_u64()
            })
            .collect();
        let b: Vec<u64> = (0..4)
            .map({
                let mut r = SplitMix64::new(1);
                move |_| r.next_u64()
            })
            .collect();
        let c: Vec<u64> = (0..4)
            .map({
                let mut r = SplitMix64::new(2);
                move |_| r.next_u64()
            })
            .collect();
        assert_eq!(a, b);
        assert_ne!(a, c);
        // Reference value for seed 0 from the published algorithm.
        assert_eq!(SplitMix64::new(0).next_u64(), 0xE220_A839_7B1D_CDAF);
    }

    #[test]
    fn floats_and_chances_are_in_range() {
        let mut r = SplitMix64::new(9);
        let mut hits = 0;
        for _ in 0..10_000 {
            let f = r.next_f64();
            assert!((0.0..1.0).contains(&f));
            if r.chance(0.25) {
                hits += 1;
            }
        }
        assert!((2_000..3_000).contains(&hits), "{hits}");
        assert!(!r.chance(0.0));
        assert!(r.chance(1.0));
        assert!(r.chance(7.0), "clamped");
    }

    #[test]
    fn fill_handles_partial_chunks() {
        let mut a = [0u8; 13];
        SplitMix64::new(3).fill(&mut a);
        assert!(a.iter().any(|&b| b != 0));
        let mut b = [0u8; 13];
        SplitMix64::new(3).fill(&mut b);
        assert_eq!(a, b);
    }
}
