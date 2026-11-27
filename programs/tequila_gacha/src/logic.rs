//! Pure draw and stock math, shared by the callback and `claim`.

pub const MAX_TIERS: usize = 6;

/// x = r mod total; the tier is the first i with x < cumulative weight. None if total = 0.
pub fn draw(weights: &[u32; MAX_TIERS], r: u64) -> Option<u8> {
    let total: u64 = weights.iter().map(|w| *w as u64).sum();
    if total == 0 {
        return None;
    }
    let x = r % total;
    let mut acc = 0u64;
    for (i, w) in weights.iter().enumerate() {
        acc += *w as u64;
        if x < acc {
            return Some(i as u8);
        }
    }
    None
}

/// Sell a pack of `count` only while `count` units are not reserved by paid, unclaimed pulls.
pub fn sellable(remaining: &[u32; MAX_TIERS], pending: u32, count: u8) -> bool {
    let total: u64 = remaining.iter().map(|w| *w as u64).sum();
    total >= pending as u64 + count as u64 && count > 0
}

pub const MAX_PACK: usize = 5;
/// Unused slots of `Pull.tiers`.
pub const NO_PICK: u8 = u8::MAX;
pub const PICK: &[u8] = b"pick";
pub const REROLL: &[u8] = b"reroll";

/// u64 LE of sha256(randomness ‖ tag ‖ index): the roll of pack index `i`.
pub fn indexed_roll(randomness: &[u8; 32], tag: &[u8], i: u8) -> u64 {
    let h = solana_sha256_hasher::hashv(&[randomness, tag, &[i]]).to_bytes();
    let mut b = [0u8; 8];
    b.copy_from_slice(&h[..8]);
    u64::from_le_bytes(b)
}

/// The pack's tiers: pick i draws from the snapshot copy with indexed_roll(PICK, i), then takes
/// that unit out of the copy (without replacement). None if the copy has fewer than `count`.
pub fn draw_pack(snapshot: &[u32; MAX_TIERS], randomness: &[u8; 32], count: u8) -> Option<[u8; MAX_PACK]> {
    let mut copy = *snapshot;
    let mut tiers = [NO_PICK; MAX_PACK];
    for i in 0..count {
        let t = draw(&copy, indexed_roll(randomness, PICK, i))?;
        copy[t as usize] -= 1;
        tiers[i as usize] = t;
    }
    Some(tiers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_walks_cumulative_weights() {
        let w = [3, 0, 2, 0, 0, 5];
        let tiers: Vec<u8> = (0..10u64).map(|x| draw(&w, x).unwrap()).collect();
        assert_eq!(tiers, vec![0, 0, 0, 2, 2, 5, 5, 5, 5, 5]);
    }

    #[test]
    fn draw_reduces_modulo_total() {
        let w = [3, 0, 2, 0, 0, 5];
        assert_eq!(draw(&w, 10), Some(0));
        assert_eq!(draw(&w, 13), Some(2));
        assert_eq!(draw(&w, u64::MAX), draw(&w, u64::MAX % 10));
    }

    #[test]
    fn draw_never_picks_an_empty_tier() {
        let w = [0, 7, 0, 0, 1, 0];
        for x in 0..1000u64 {
            let t = draw(&w, x).unwrap() as usize;
            assert!(w[t] > 0);
        }
    }

    #[test]
    fn draw_with_no_weight_is_none() {
        assert_eq!(draw(&[0; 6], 42), None);
    }

    #[test]
    fn sellable_needs_count_unreserved_units() {
        assert!(sellable(&[1, 0, 0, 0, 0, 0], 0, 1));
        assert!(!sellable(&[1, 0, 0, 0, 0, 0], 1, 1));
        assert!(sellable(&[2, 3, 0, 0, 0, 0], 4, 1));
        assert!(!sellable(&[2, 3, 0, 0, 0, 0], 5, 1));
        assert!(!sellable(&[0; 6], 0, 1));
        assert!(sellable(&[2, 3, 0, 0, 0, 0], 2, 3));
        assert!(!sellable(&[2, 3, 0, 0, 0, 0], 3, 3));
        assert!(sellable(&[5, 0, 0, 0, 0, 0], 0, 5));
        assert!(!sellable(&[4, 0, 0, 0, 0, 0], 0, 5));
    }

    fn sha_u64(r: &[u8; 32], tag: &[u8], i: u8) -> u64 {
        let h = solana_sha256_hasher::hashv(&[r, tag, &[i]]).to_bytes();
        u64::from_le_bytes(h[..8].try_into().unwrap())
    }

    #[test]
    fn indexed_roll_is_sha256_of_randomness_tag_index() {
        let r = [9u8; 32];
        for i in 0..5u8 {
            assert_eq!(indexed_roll(&r, PICK, i), sha_u64(&r, b"pick", i));
            assert_eq!(indexed_roll(&r, REROLL, i), sha_u64(&r, b"reroll", i));
        }
        assert_ne!(indexed_roll(&r, PICK, 0), indexed_roll(&r, PICK, 1));
    }

    #[test]
    fn pack_draws_without_replacement_from_the_copy() {
        for seed in 0..200u8 {
            let r = [seed; 32];
            let mut tiers = draw_pack(&[1, 0, 1, 0, 0, 1], &r, 3).unwrap();
            tiers[..3].sort();
            assert_eq!(&tiers[..3], &[0, 2, 5], "each unit once");
            let t = draw_pack(&[2, 1, 0, 0, 0, 0], &r, 3).unwrap();
            assert_eq!(t[..3].iter().filter(|x| **x == 0).count(), 2);
        }
    }

    #[test]
    fn pack_pick_i_uses_the_copy_left_by_the_previous_picks() {
        let r = [3u8; 32];
        let snap = [3, 0, 2, 0, 0, 5];
        let tiers = draw_pack(&snap, &r, 5).unwrap();
        let mut copy = snap;
        for i in 0..5u8 {
            let t = draw(&copy, sha_u64(&r, b"pick", i)).unwrap();
            assert_eq!(tiers[i as usize], t);
            copy[t as usize] -= 1;
        }
    }

    #[test]
    fn pack_count_one_is_the_first_pick() {
        let r = [77u8; 32];
        let snap = [3, 0, 2, 0, 0, 5];
        let t = draw_pack(&snap, &r, 1).unwrap();
        assert_eq!(t[0], draw(&snap, sha_u64(&r, b"pick", 0)).unwrap());
        assert_eq!(&t[1..], &[NO_PICK; 4]);
    }

    #[test]
    fn pack_larger_than_the_copy_is_none() {
        assert_eq!(draw_pack(&[1, 1, 0, 0, 0, 0], &[0; 32], 3), None);
        assert_eq!(draw_pack(&[0; 6], &[0; 32], 1), None);
    }
}
