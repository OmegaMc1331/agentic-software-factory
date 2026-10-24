/// Conventional median of an ascending-sorted sample of u64 values: the
/// middle element for odd-sized samples, and the arithmetic mean of the two
/// central elements for even-sized samples. The mean rounds to the nearest
/// unit with an exact half rounded up (`[10, 11] -> 11`), is computed through
/// u128 so it cannot overflow, and always fits the u64 range because the mean
/// of two u64 values is never above the larger one. Empty samples yield
/// `None`; the input slice is only read.
pub fn median_u64(sorted: &[u64]) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        Some(sorted[middle])
    } else {
        let sum = u128::from(sorted[middle - 1]) + u128::from(sorted[middle]);
        Some(u64::try_from(sum.div_ceil(2)).expect("mean of two u64 values fits u64"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn median_of(mut sample: Vec<u64>) -> Option<u64> {
        sample.sort_unstable();
        median_u64(&sample)
    }

    #[test]
    fn empty_sample_has_no_median() {
        assert_eq!(median_u64(&[]), None);
    }

    #[test]
    fn single_element_sample_returns_that_element() {
        assert_eq!(median_of(vec![10]), Some(10));
    }

    #[test]
    fn two_element_samples_average_with_half_rounded_up() {
        assert_eq!(median_of(vec![10, 20]), Some(15));
        assert_eq!(median_of(vec![10, 11]), Some(11));
        assert_eq!(median_of(vec![0, 1]), Some(1));
        assert_eq!(median_of(vec![0, 0]), Some(0));
    }

    #[test]
    fn odd_samples_return_the_middle_element() {
        assert_eq!(median_of(vec![10, 20, 30]), Some(20));
        assert_eq!(median_of(vec![7, 3, 5, 1, 9]), Some(5));
    }

    #[test]
    fn even_samples_average_the_two_central_elements() {
        assert_eq!(median_of(vec![10, 20, 30, 40]), Some(25));
        assert_eq!(median_of(vec![1, 2, 3, 4, 5, 6]), Some(4));
        assert_eq!(median_of(vec![21, 22]), Some(22));
    }

    #[test]
    fn duplicate_and_identical_samples_are_stable() {
        assert_eq!(median_of(vec![5, 5, 7, 7]), Some(6));
        assert_eq!(median_of(vec![5, 5, 5, 5]), Some(5));
        assert_eq!(median_of(vec![0, 0, 0]), Some(0));
    }

    #[test]
    fn unsorted_input_is_sorted_by_the_caller_contract() {
        let mut sample = vec![40, 10, 30, 20];
        sample.sort_unstable();
        assert_eq!(median_u64(&sample), Some(25));
    }

    #[test]
    fn extreme_values_do_not_overflow_or_panic() {
        let max = u64::MAX;
        assert_eq!(median_of(vec![max]), Some(max));
        assert_eq!(median_of(vec![max, max]), Some(max));
        assert_eq!(median_of(vec![max - 1, max]), Some(max));
        assert_eq!(
            median_of(vec![0, max]),
            Some(u128::from(max).div_ceil(2) as u64),
            "an exact half rounds up, matching [0, 1] -> 1 at u64 scale"
        );
        assert_eq!(
            median_of(vec![max - 3, max - 2, max - 1, max]),
            Some((u128::from(max - 2) + u128::from(max - 1)).div_ceil(2) as u64)
        );
        assert_eq!(
            median_of(vec![max - 1, max, max - 2, max - 3]),
            Some((u128::from(max - 2) + u128::from(max - 1)).div_ceil(2) as u64)
        );
    }

    #[test]
    fn source_sample_is_not_mutated() {
        let sample = vec![40, 10, 30, 20, 5];
        let before = sample.clone();
        let mut sorted = sample.clone();
        sorted.sort_unstable();
        let _ = median_u64(&sorted);
        assert_eq!(sample, before);
    }
}
