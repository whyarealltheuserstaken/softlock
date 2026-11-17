//! Timer-tick drift and App Nap / throttle verdicts.
//!
//! App Nap has no supported "is this process napping?" getter that this kit
//! can trust. Occluded, idle apps have their timers coalesced, so wake
//! latency and inter-tick gaps are the measurement.

pub const APP_NAP_GAP_THRESHOLD_MS: u64 = 1_000;
pub const APP_NAP_MIN_SAMPLE_MS: u64 = 50_000;
pub const APP_NAP_THRESHOLD_WHY: &str = "App Nap and timer coalescing delay an occluded idle process by a second or more; a healthy 50 ms timer stays within a few hundred milliseconds of its step";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriftStats {
    pub tick_count: u64,
    pub expected_ticks: u64,
    pub max_gap_ms: u64,
    pub p95_gap_ms: u64,
    pub mean_gap_ms: u64,
    pub max_wake_late_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NapVerdict {
    pub napped: Option<bool>,
    pub token: &'static str,
    pub reason: String,
}

pub fn summarize_gaps(gaps_ms: &[u64]) -> (u64, u64, u64) {
    if gaps_ms.is_empty() {
        return (0, 0, 0);
    }
    let mut sorted = gaps_ms.to_vec();
    sorted.sort_unstable();
    let max = *sorted.last().unwrap_or(&0);
    let p95 = percentile(&sorted, 95);
    let sum: u128 = sorted.iter().map(|value| u128::from(*value)).sum();
    let mean = (sum / sorted.len() as u128) as u64;
    (max, p95, mean)
}

pub fn percentile(sorted_ascending: &[u64], percent: u8) -> u64 {
    if sorted_ascending.is_empty() {
        return 0;
    }
    let percent = u64::from(percent.min(100));
    let last = (sorted_ascending.len() - 1) as u64;
    let idx = (last * percent).div_ceil(100);
    sorted_ascending[idx as usize]
}

pub fn stats_from_gaps(
    tick_count: u64,
    expected_ticks: u64,
    gaps_ms: &[u64],
    wake_late_ms: &[u64],
) -> DriftStats {
    let (max_gap_ms, p95_gap_ms, mean_gap_ms) = summarize_gaps(gaps_ms);
    let max_wake_late_ms = wake_late_ms.iter().copied().max().unwrap_or(0);
    DriftStats {
        tick_count,
        expected_ticks,
        max_gap_ms,
        p95_gap_ms,
        mean_gap_ms,
        max_wake_late_ms,
    }
}

pub fn nap_verdict(stats: &DriftStats, sample_ms: u64) -> NapVerdict {
    if sample_ms < APP_NAP_MIN_SAMPLE_MS {
        return NapVerdict {
            napped: None,
            token: "not-run-on-runner",
            reason: format!(
                "sample {sample_ms} ms is shorter than {APP_NAP_MIN_SAMPLE_MS} ms, so App Nap is not claimed"
            ),
        };
    }
    if stats.tick_count == 0 {
        return NapVerdict {
            napped: None,
            token: "not-claimed",
            reason: "no ticks were recorded, so App Nap is not claimed".to_string(),
        };
    }
    let gap_hit = stats.max_gap_ms >= APP_NAP_GAP_THRESHOLD_MS;
    let late_hit = stats.max_wake_late_ms >= APP_NAP_GAP_THRESHOLD_MS;
    let count_hit = stats.expected_ticks > 0 && stats.tick_count * 100 < stats.expected_ticks * 50;
    let napped = gap_hit || late_hit || count_hit;
    let token = if napped { "napped" } else { "not-napped" };
    let reason = format!(
        "{token}: max-gap={}ms p95-gap={}ms mean-gap={}ms max-wake-late={}ms ticks={}/{} threshold={}ms because {APP_NAP_THRESHOLD_WHY}",
        stats.max_gap_ms,
        stats.p95_gap_ms,
        stats.mean_gap_ms,
        stats.max_wake_late_ms,
        stats.tick_count,
        stats.expected_ticks,
        APP_NAP_GAP_THRESHOLD_MS
    );
    NapVerdict {
        napped: Some(napped),
        token,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_and_summary_cover_empty_and_skewed_samples() {
        assert_eq!(summarize_gaps(&[]), (0, 0, 0));
        assert_eq!(percentile(&[], 95), 0);
        let gaps = [50_u64, 51, 52, 53, 54, 55, 56, 57, 58, 2000];
        let (max, p95, mean) = summarize_gaps(&gaps);
        assert_eq!(max, 2000);
        assert_eq!(p95, 2000);
        assert!(mean > 50 && mean < 2000);
        assert_eq!(percentile(&[10, 20, 30], 0), 10);
        assert_eq!(percentile(&[10, 20, 30], 100), 30);
    }

    #[test]
    fn a_one_second_gap_counts_as_napped() {
        let stats = stats_from_gaps(100, 1000, &[55, 60, 1_200], &[2, 8]);
        let verdict = nap_verdict(&stats, 60_000);
        assert_eq!(verdict.napped, Some(true));
        assert_eq!(verdict.token, "napped");
        assert!(verdict.reason.contains("threshold=1000ms"));
        assert!(verdict.reason.contains("App Nap"));
        assert_eq!(stats.max_gap_ms, 1_200);
        assert_eq!(stats.max_wake_late_ms, 8);
    }

    #[test]
    fn regular_ticks_are_not_napped() {
        let gaps: Vec<u64> = (0..80).map(|_| 60).collect();
        let late: Vec<u64> = (0..80).map(|_| 4).collect();
        let stats = stats_from_gaps(80, 100, &gaps, &late);
        let verdict = nap_verdict(&stats, 60_000);
        assert_eq!(verdict.napped, Some(false));
        assert_eq!(verdict.token, "not-napped");
        assert!(verdict.reason.contains("max-gap=60ms"));
    }

    #[test]
    fn a_late_wake_or_half_count_also_counts_as_napped() {
        let late = stats_from_gaps(90, 100, &[60], &[1_500]);
        assert_eq!(nap_verdict(&late, 60_000).napped, Some(true));
        let slow = stats_from_gaps(40, 100, &[60], &[3]);
        assert_eq!(nap_verdict(&slow, 60_000).napped, Some(true));
    }

    #[test]
    fn short_or_empty_samples_are_not_claimed() {
        let stats = stats_from_gaps(20, 133, &[60], &[1]);
        let runner = nap_verdict(&stats, 8_000);
        assert_eq!(runner.napped, None);
        assert_eq!(runner.token, "not-run-on-runner");
        let empty = stats_from_gaps(0, 1000, &[], &[]);
        let missing = nap_verdict(&empty, 60_000);
        assert_eq!(missing.napped, None);
        assert_eq!(missing.token, "not-claimed");
    }
}
