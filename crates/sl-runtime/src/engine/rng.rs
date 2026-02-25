pub(super) fn next_random_u32(state: &mut u32) -> u32 {
    let mut next = state.wrapping_add(0x6d2b79f5);
    *state = next;
    next = (next ^ (next >> 15)).wrapping_mul(next | 1);
    next ^= next.wrapping_add((next ^ (next >> 7)).wrapping_mul(next | 61));
    next ^ (next >> 14)
}

pub(super) fn next_random_bounded(state: &mut u32, bound: u32) -> u32 {
    next_random_bounded_with(state, bound, next_random_u32)
}

pub(super) fn next_random_bounded_with<F>(state: &mut u32, bound: u32, mut next: F) -> u32
where
    F: FnMut(&mut u32) -> u32,
{
    let threshold = (u64::from(u32::MAX) + 1) / u64::from(bound) * u64::from(bound);
    let mut candidate = next(state);
    while u64::from(candidate) >= threshold {
        candidate = next(state);
    }
    candidate % bound
}

#[cfg(test)]
mod rng_tests {
    use super::*;

    #[test]
    fn next_random_bounded_with_covers_threshold_retry_path() {
        let mut state = 0u32;
        let mut values = vec![u32::MAX, 42u32].into_iter();
        let result = next_random_bounded_with(&mut state, 10, |_s| {
            values.next().expect("test values should be available")
        });
        assert_eq!(result, 2);
    }
}
