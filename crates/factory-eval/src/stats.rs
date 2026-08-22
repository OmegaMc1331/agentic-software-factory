/// Minimum sample size before a rate is presented as reliable. Below this
/// threshold the summary keeps the raw numbers but is flagged unreliable and
/// the dashboard renders "Insufficient data" instead of a percentage.
pub const MIN_RELIABLE_RATE_SAMPLES: u64 = 10;

/// Minimum sample size before a duration median/p95 is considered stable
/// enough to display without a caveat.
pub const MIN_RELIABLE_DURATION_SAMPLES: u64 = 5;

/// z-value for a 95% Wilson score interval.
const WILSON_Z: f64 = 1.96;

/// A proportion with its 95% Wilson confidence interval. `total == 0` yields
/// `None` rate/interval; `reliable` requires
/// [`MIN_RELIABLE_RATE_SAMPLES`] samples.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateStats {
    pub successes: u64,
    pub total: u64,
    /// Success fraction in `[0, 1]`, or `None` when there is no sample.
    pub rate: Option<f64>,
    /// Lower/upper Wilson bound, `None` when there is no sample.
    pub interval_low: Option<f64>,
    pub interval_high: Option<f64>,
    pub reliable: bool,
}

impl RateStats {
    pub fn new(successes: u64, total: u64) -> Self {
        let rate = (total > 0).then(|| successes as f64 / total as f64);
        let (interval_low, interval_high) = (total > 0)
            .then(|| wilson_95(successes, total))
            .map(|(low, high)| (Some(low), Some(high)))
            .unwrap_or((None, None));
        RateStats {
            successes,
            total,
            rate,
            interval_low,
            interval_high,
            reliable: total >= MIN_RELIABLE_RATE_SAMPLES,
        }
    }
}

/// Wilson score interval at 95% for `successes` out of `total` (total > 0).
/// Both bounds are clamped to `[0, 1]`.
pub fn wilson_95(successes: u64, total: u64) -> (f64, f64) {
    let n = total as f64;
    let p = successes as f64 / n;
    let z2 = WILSON_Z * WILSON_Z;
    let denominator = 1.0 + z2 / n;
    let center = (p + z2 / (2.0 * n)) / denominator;
    let spread = WILSON_Z * ((p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / denominator);
    (
        (center - spread).clamp(0.0, 1.0),
        (center + spread).clamp(0.0, 1.0),
    )
}

/// Median and p95 of a duration sample, in milliseconds. The median of an
/// even-sized sample is the mean of the two central values (rounded to the
/// nearest millisecond); p95 uses the nearest-rank method. Samples derived
/// from attempt wall-time rather than session timers are counted separately
/// in `approximate_samples` so consumers can disclose the approximation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DurationStats {
    pub samples: u64,
    pub median_ms: Option<u64>,
    pub p95_ms: Option<u64>,
    pub approximate_samples: u64,
    pub reliable: bool,
}

impl DurationStats {
    pub fn from_samples_ms(samples: &[u64], approximate_samples: u64) -> Self {
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        DurationStats {
            samples: sorted.len() as u64,
            median_ms: median(&sorted),
            p95_ms: p95(&sorted),
            approximate_samples: approximate_samples.min(sorted.len() as u64),
            reliable: sorted.len() as u64 >= MIN_RELIABLE_DURATION_SAMPLES,
        }
    }
}

fn median(sorted: &[u64]) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let middle = sorted.len() / 2;
    Some(if sorted.len() % 2 == 1 {
        sorted[middle]
    } else {
        (sorted[middle - 1] + sorted[middle]) / 2
    })
}

fn p95(sorted: &[u64]) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    // nearest-rank: the ceil(0.95 * n)-th smallest value
    let rank = ((sorted.len() as f64) * 0.95).ceil() as usize;
    let index = rank.clamp(1, sorted.len()) - 1;
    Some(sorted[index])
}
