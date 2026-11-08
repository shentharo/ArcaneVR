//! Counts alternating left/right steps in a walk-in-place recording.
//! Input is (time, acceleration magnitude) per foot. Each foot's own median is
//! removed first, so a gravity baseline in the sensor data does not matter.
//! Thresholds are untuned against live walking: treat the count as an estimate.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StepCount {
    pub total: usize,
    pub per_foot: [usize; 2],
}

const SMOOTHING_SECONDS: f64 = 0.08;
const MIN_PEAK: f64 = 1.0;
const MIN_PEAK_SPACING: f64 = 0.32;
const MIN_STEP_INTERVAL: f64 = 0.2;

fn peaks(samples: &[(f64, f64)]) -> Vec<f64> {
    if samples.len() < 3 {
        return Vec::new();
    }
    let mut sorted: Vec<f64> = samples.iter().map(|s| s.1).filter(|m| m.is_finite()).collect();
    if sorted.is_empty() {
        return Vec::new();
    }
    sorted.sort_by(f64::total_cmp);
    let baseline = sorted[sorted.len() / 2];
    let mut smooth = 0.;
    let mut previous: Option<f64> = None;
    let mut series = Vec::with_capacity(samples.len());
    for &(t, m) in samples {
        if !t.is_finite() || !m.is_finite() || previous.is_some_and(|p| t <= p) {
            continue;
        }
        let dt = previous.map_or(0.01, |p| (t - p).clamp(0., 0.2));
        previous = Some(t);
        smooth += ((m - baseline).abs() - smooth) * (1. - (-dt / SMOOTHING_SECONDS).exp());
        series.push((t, smooth));
    }
    let top = series.iter().map(|s| s.1).fold(0., f64::max);
    let threshold = MIN_PEAK.max(top * 0.35);
    let (mut peak, mut peak_at) = (0., 0.);
    let mut last: Option<f64> = None;
    let mut found = Vec::new();
    for (t, s) in series {
        if s > peak {
            peak = s;
            peak_at = t;
        }
        if s < peak * 0.5 {
            if peak >= threshold && last.is_none_or(|l| peak_at - l >= MIN_PEAK_SPACING) {
                found.push(peak_at);
                last = Some(peak_at);
            }
            peak = s;
            peak_at = t;
        }
    }
    found
}

/// Steps are acceleration peaks that alternate between the two feet.
/// A repeat from the same foot, or a peak under 0.2 s after the last step, is ignored.
pub fn count_alternating_steps(feet: [&[(f64, f64)]; 2]) -> StepCount {
    let mut events: Vec<(f64, usize)> = Vec::new();
    for (side, samples) in feet.iter().enumerate() {
        events.extend(peaks(samples).into_iter().map(|t| (t, side)));
    }
    events.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut count = StepCount::default();
    let mut last: Option<(f64, usize)> = None;
    for (t, side) in events {
        if last.is_none_or(|(at, previous)| previous != side && t - at >= MIN_STEP_INTERVAL) {
            count.total += 1;
            count.per_foot[side] += 1;
            last = Some((t, side));
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feet(baseline: f64, left_at: &[f64], right_at: &[f64], height: f64, seconds: f64) -> [Vec<(f64, f64)>; 2] {
        let make = |bumps: &[f64]| -> Vec<(f64, f64)> {
            (0..(seconds * 100.) as usize)
                .map(|n| {
                    let t = n as f64 * 0.01;
                    let bump: f64 = bumps.iter().map(|b| height * (-((t - b) / 0.08).powi(2)).exp()).sum();
                    (t, baseline + bump)
                })
                .collect()
        };
        [make(left_at), make(right_at)]
    }
    fn count(f: &[Vec<(f64, f64)>; 2]) -> StepCount {
        count_alternating_steps([&f[0], &f[1]])
    }

    #[test]
    fn ten_alternating_steps_are_counted() {
        let left: Vec<f64> = (0..5).map(|n| 0.5 + n as f64 * 2.).collect();
        let right: Vec<f64> = (0..5).map(|n| 1.5 + n as f64 * 2.).collect();
        let c = count(&feet(0., &left, &right, 4., 10.));
        assert_eq!(c, StepCount { total: 10, per_foot: [5, 5] });
    }
    #[test]
    fn gravity_baseline_does_not_change_the_count() {
        let left: Vec<f64> = (0..5).map(|n| 0.5 + n as f64 * 2.).collect();
        let right: Vec<f64> = (0..5).map(|n| 1.5 + n as f64 * 2.).collect();
        assert_eq!(count(&feet(9.81, &left, &right, 4., 10.)).total, 10);
    }
    #[test]
    fn standing_still_and_tiny_noise_count_nothing() {
        assert_eq!(count(&feet(9.81, &[], &[], 0., 10.)).total, 0);
        let left: Vec<f64> = (0..5).map(|n| 0.5 + n as f64 * 2.).collect();
        assert_eq!(count(&feet(9.81, &left, &left, 0.4, 10.)).total, 0);
    }
    #[test]
    fn one_foot_only_is_not_alternating_steps() {
        let left: Vec<f64> = (0..5).map(|n| 0.5 + n as f64 * 2.).collect();
        let c = count(&feet(0., &left, &[], 4., 10.));
        assert_eq!(c.total, 1);
        assert_eq!(c.per_foot[1], 0);
    }
    #[test]
    fn fast_running_steps_are_still_counted() {
        let left: Vec<f64> = (0..8).map(|n| 0.4 + n as f64 * 0.8).collect();
        let right: Vec<f64> = (0..8).map(|n| 0.8 + n as f64 * 0.8).collect();
        assert!(count(&feet(0., &left, &right, 6., 7.)).total >= 14);
    }
    #[test]
    fn empty_and_bad_data_is_safe() {
        assert_eq!(count_alternating_steps([&[], &[]]), StepCount::default());
        let bad = [(0., f64::NAN), (0.1, f64::INFINITY), (0.2, 1.)];
        assert_eq!(count_alternating_steps([&bad, &bad]).total, 0);
    }
}
