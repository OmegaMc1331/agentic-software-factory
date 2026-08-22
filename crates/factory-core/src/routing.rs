//! Deterministic, explainable agent routing.
//!
//! This module owns the *scoring* half of routing; the scheduler
//! (`factory::Factory`) owns candidate filtering, capacity reservation, and
//! persistence of [`RoutingDecision`] audit records. There is no LLM and no
//! randomness anywhere in the path: given the same candidate facts, the same
//! durable performance data, and the same capacity state, the router always
//! produces the same ranking.
//!
//! # Routing score
//!
//! A candidate is scored only when `factory-eval` holds a *reliable*
//! performance slice for it (see [`factory_eval::resolve_performance`]);
//! candidates without reliable history are never ranked from thin samples.
//! For a reliable candidate the score is
//!
//! ```text
//! score = 0.55 * quality        Wilson-LB mean of first-pass and eventual
//!                                approval (evidence quality is rewarded:
//!                                a conservative lower bound, so 97% with
//!                                n=10 does not beat 94% with n=200)
//!       + 0.20 * rework         1 - Wilson-UB of the retry rate
//!       + 0.10 * speed          slowest reliable median / own median,
//!                                clamped to [0, 1]; 0.5 (neutral) when the
//!                                duration sample is not yet reliable
//!       + 0.15 * capacity       free fraction of the agent's max_concurrency
//!       + 0.02                  preferred-agent bonus (breaks near-ties,
//!                                never overrides a real performance gap)
//!       - 0.05                  retry penalty for agents whose previous
//!                                attempt on THIS task failed or was asked
//!                                for changes (retries may route elsewhere)
//! ```
//!
//! Quality deliberately dominates speed: a fast but unreliable agent cannot
//! out-rank a slower high-quality one. Integration conflicts stay a separate
//! weak signal inside `factory-eval` and never enter this score.
//!
//! # Tie-breaking
//!
//! Score (descending), then preferred, then the pool's configured position,
//! then agent name. HashMap iteration order is never consulted.
//!
//! # Cold start and exploration
//!
//! When some candidates are unranked but at least one is ranked, every
//! `EXPLORATION_INTERVAL`-th dispatch (driven by the durable run attempt
//! count) is routed to the least-observed unranked candidate with free
//! capacity, so no agent can monopolize all future evidence. This is a fixed
//! deterministic rule, not bandit infrastructure.

use factory_eval::{ResolvedPerformance, MIN_RELIABLE_RATE_SAMPLES};
use factory_types::{RoutingCandidateScore, RoutingMode};

/// Weight of the quality component (first-pass/eventual approval).
pub const WEIGHT_QUALITY: f64 = 0.55;
/// Weight of the rework-efficiency component (retry rate).
pub const WEIGHT_REWORK: f64 = 0.20;
/// Weight of the execution-duration component.
pub const WEIGHT_SPEED: f64 = 0.10;
/// Weight of the current-capacity component.
pub const WEIGHT_CAPACITY: f64 = 0.15;
/// Small deterministic bonus for the role's preferred agent.
pub const PREFERRED_BONUS: f64 = 0.02;
/// Penalty applied to candidates that already failed (or were asked for
/// changes) on the specific task being routed, so retries can move on.
pub const RETRY_PENALTY: f64 = 0.05;
/// Speed component used when an agent's duration sample is not reliable.
pub const NEUTRAL_SPEED: f64 = 0.5;
/// One in every N dispatches may explore an under-sampled candidate.
pub const EXPLORATION_INTERVAL: usize = 5;

/// Everything the router needs to know about one filtered candidate.
#[derive(Debug, Clone)]
pub struct CandidateFacts {
    pub agent: String,
    /// Whether this agent is the role's preferred assignment.
    pub preferred: bool,
    /// The agent's most specific *reliable* performance slice, or `None`
    /// when no slice has enough qualifying samples to be ranked.
    pub performance: Option<ResolvedPerformance>,
    /// All-time tasks attributed to the agent (any role), used only to order
    /// exploration of under-sampled candidates.
    pub observed_tasks: u64,
    /// In-flight invocations at scoring time.
    pub inflight: u64,
    /// The agent's `max_concurrency` (>= 1).
    pub limit: u64,
    /// A previous attempt on this task by this agent failed or received a
    /// request-changes decision.
    pub prior_rejection: bool,
}

impl CandidateFacts {
    pub fn free_slots(&self) -> u64 {
        self.limit.max(1) - self.inflight.min(self.limit.max(1))
    }
}

/// One candidate after scoring, in rank order (ranked candidates first,
/// unranked candidates keep pool order after them).
#[derive(Debug, Clone, PartialEq)]
pub struct RankedCandidate {
    pub agent: String,
    pub score: Option<f64>,
    pub reliable: bool,
    pub note: String,
    pub preferred: bool,
    pub free_slots: u64,
    pub observed_tasks: u64,
}

/// Scores and orders the candidates deterministically. See the module docs
/// for the formula; the inputs are pure facts so the function is trivially
/// testable and stable.
pub fn rank(candidates: &[CandidateFacts]) -> Vec<RankedCandidate> {
    let speed_reference = candidates
        .iter()
        .filter_map(|facts| reliable_median_ms(&facts.performance))
        .max()
        .unwrap_or(0);

    let mut ranked: Vec<(usize, RankedCandidate)> = candidates
        .iter()
        .enumerate()
        .map(|(position, facts)| {
            let score = facts
                .performance
                .as_ref()
                .map(|resolved| score_candidate(facts, resolved, speed_reference));
            let note = match (&facts.performance, score) {
                (Some(resolved), _) => format!(
                    "{} slice, n={}",
                    resolved.level.as_str(),
                    resolved.sample_count()
                ),
                (None, _) => format!(
                    "insufficient data (n={} of {MIN_RELIABLE_RATE_SAMPLES})",
                    facts.observed_tasks
                ),
            };
            (
                position,
                RankedCandidate {
                    agent: facts.agent.clone(),
                    score,
                    reliable: facts.performance.is_some(),
                    note,
                    preferred: facts.preferred,
                    free_slots: facts.free_slots(),
                    observed_tasks: facts.observed_tasks,
                },
            )
        })
        .collect();

    // Scored candidates first (score desc, preferred, pool position, name);
    // unscored candidates afterwards in pool order. `total_cmp` keeps the
    // sort total and deterministic for float scores.
    ranked.sort_by(|(pos_a, a), (pos_b, b)| compare_ranked(a, b, *pos_a, *pos_b));
    ranked.into_iter().map(|(_, ranked)| ranked).collect()
}

fn compare_ranked(
    a: &RankedCandidate,
    b: &RankedCandidate,
    pos_a: usize,
    pos_b: usize,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    // Reliable candidates outrank unreliable ones.
    match (a.score, b.score) {
        (Some(_), None) => return Ordering::Less,
        (None, Some(_)) => return Ordering::Greater,
        _ => {}
    }
    if let (Some(sa), Some(sb)) = (a.score, b.score) {
        match sa.total_cmp(&sb) {
            Ordering::Equal => {}
            order => return order.reverse(), // higher score first
        }
    }
    // Preferred before non-preferred.
    match (a.preferred, b.preferred) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }
    // Configured pool position, then agent name.
    pos_a.cmp(&pos_b).then_with(|| a.agent.cmp(&b.agent))
}

/// The deterministic routing score of one candidate. See module docs.
fn score_candidate(
    facts: &CandidateFacts,
    resolved: &ResolvedPerformance,
    speed_reference_ms: u64,
) -> f64 {
    let metrics = &resolved.metrics;
    let first_pass_lb = metrics.first_pass_approval.interval_low.unwrap_or(0.0);
    let eventual_lb = metrics.eventual_approval.interval_low.unwrap_or(0.0);
    let quality = (first_pass_lb + eventual_lb) / 2.0;
    // Conservative rework view: assume retries are as common as the upper
    // Wilson bound allows.
    let retry_ub = metrics.retry_rate.interval_high.unwrap_or(1.0);
    let rework = (1.0 - retry_ub).clamp(0.0, 1.0);
    let speed = match reliable_median_ms(&Some(resolved.clone())) {
        Some(median) if speed_reference_ms > 0 => {
            (speed_reference_ms as f64 / median as f64).clamp(0.0, 1.0)
        }
        _ => NEUTRAL_SPEED,
    };
    let limit = facts.limit.max(1);
    let capacity = (((limit - facts.inflight.min(limit)) as f64) / (limit as f64)).clamp(0.0, 1.0);
    let mut score = WEIGHT_QUALITY * quality
        + WEIGHT_REWORK * rework
        + WEIGHT_SPEED * speed
        + WEIGHT_CAPACITY * capacity;
    if facts.preferred {
        score += PREFERRED_BONUS;
    }
    if facts.prior_rejection {
        score -= RETRY_PENALTY;
    }
    score
}

fn reliable_median_ms(performance: &Option<ResolvedPerformance>) -> Option<u64> {
    let resolved = performance.as_ref()?;
    let duration = &resolved.metrics.execution_duration;
    (duration.reliable).then_some(duration.median_ms).flatten()
}

/// The deterministic exploration pick, if this dispatch should explore.
///
/// Explores only when exploration is enabled, at least one candidate is
/// reliably ranked, at least one is not, the rotation counter hits the
/// interval, and some unranked candidate has free capacity. The least
/// observed unranked candidate wins (pool position breaks ties), so cold
/// candidates gather data at a bounded, predictable rate.
pub fn exploration_pick(
    candidates: &[CandidateFacts],
    ranked: &[RankedCandidate],
    mode: RoutingMode,
    exploration_enabled: bool,
    rotation_index: usize,
) -> Option<String> {
    if mode != RoutingMode::Performance || !exploration_enabled {
        return None;
    }
    if !rotation_index.is_multiple_of(EXPLORATION_INTERVAL) {
        return None;
    }
    let any_ranked = ranked.iter().any(|candidate| candidate.reliable);
    if !any_ranked {
        // Nobody is ranked: plain round-robin fallback already spreads work.
        return None;
    }
    candidates
        .iter()
        .filter(|facts| facts.performance.is_none() && facts.free_slots() > 0)
        .min_by_key(|facts| (facts.observed_tasks, facts.agent.clone()))
        .map(|facts| facts.agent.clone())
}

/// Converts ranked candidates to the durable audit representation.
pub fn candidate_scores(ranked: &[RankedCandidate]) -> Vec<RoutingCandidateScore> {
    ranked
        .iter()
        .map(|candidate| RoutingCandidateScore {
            agent: candidate.agent.clone(),
            score: candidate.score,
            reliable: candidate.reliable,
            note: candidate.note.clone(),
        })
        .collect()
}

/// Short reason strings persisted in routing decisions. Kept as constants so
/// audit records stay greppable and stable.
pub mod reasons {
    pub const OVERRIDE: &str =
        "Manual override: the pinned agent passed role, policy, and availability checks.";
    pub const SCORED: &str = "Highest reliable routing score with available capacity.";
    pub const EXPLORATION: &str = "Exploration: routing to an under-sampled eligible candidate (deterministic every-fifth dispatch).";
    pub const FALLBACK: &str =
        "Insufficient reliable performance data; capacity-aware round-robin fallback.";
    pub const ALL_SATURATED: &str =
        "All performance-ranked candidates are saturated; capacity-aware fallback.";
    pub const PREFERRED: &str = "Manual mode: the role's preferred agent is selected.";
    pub const ROUND_ROBIN: &str = "Capacity-aware round-robin selection.";
}

#[cfg(test)]
mod tests {
    use super::*;
    use factory_eval::{AgentMetrics, DurationStats, PerformanceSliceLevel, RateStats};

    fn metrics(first_pass: u64, eventual: u64, retries: u64, qualifying: u64) -> AgentMetrics {
        AgentMetrics {
            tasks_attempted: qualifying,
            attempts: qualifying + retries,
            attempts_per_task: None,
            avg_attempts_per_successful: None,
            qualifying_tasks: qualifying,
            outcome_counts: Default::default(),
            first_pass_approval: RateStats::new(first_pass, qualifying),
            eventual_approval: RateStats::new(eventual, qualifying),
            request_changes: RateStats::new(retries, qualifying),
            retry_rate: RateStats::new(retries, qualifying),
            terminal_failure: RateStats::new(0, qualifying),
            execution_duration: DurationStats::from_samples_ms(&[], 0),
            review_duration: DurationStats::from_samples_ms(&[], 0),
            total_duration: DurationStats::from_samples_ms(&[], 0),
            integration: Default::default(),
        }
    }

    fn resolved(metrics: AgentMetrics) -> Option<ResolvedPerformance> {
        Some(ResolvedPerformance {
            level: PerformanceSliceLevel::RoleOperation,
            metrics,
        })
    }

    fn facts(agent: &str, performance: Option<ResolvedPerformance>) -> CandidateFacts {
        let observed = performance
            .as_ref()
            .map(|p| p.metrics.tasks_attempted)
            .unwrap_or(0);
        CandidateFacts {
            agent: agent.to_string(),
            preferred: false,
            performance,
            observed_tasks: observed,
            inflight: 0,
            limit: 1,
            prior_rejection: false,
        }
    }

    fn reliable(agent: &str, first_pass: u64, qualifying: u64) -> CandidateFacts {
        let retries = qualifying - first_pass;
        facts(
            agent,
            resolved(metrics(first_pass, qualifying, retries, qualifying)),
        )
    }

    #[test]
    fn reliable_quality_wins_over_unreliable() {
        let ranked = rank(&[reliable("good", 12, 12), facts("cold", None)]);
        assert_eq!(ranked[0].agent, "good");
        assert!(ranked[0].reliable);
        assert!(ranked[1].score.is_none());
    }

    #[test]
    fn confidence_beats_raw_rate() {
        // 97% with n=10 must lose to 94% with n=200: the Wilson lower bound
        // of the small sample is far below the large one's.
        let small = reliable("small", 10, 10);
        let large = reliable("large", 188, 200);
        let ranked = rank(&[small, large]);
        assert_eq!(ranked[0].agent, "large");
    }

    #[test]
    fn quality_dominates_speed() {
        // fast-but-worse vs slow-but-better: the 10% speed weight cannot
        // flip a real quality gap.
        let mut worse = reliable("worse", 12, 12);
        let mut better = reliable("better", 20, 20);
        let with_duration = |facts: &mut CandidateFacts, ms: u64| {
            let mut m = facts.performance.take().unwrap();
            m.metrics.execution_duration = DurationStats::from_samples_ms(&[ms; 6], 0);
            facts.performance = Some(m);
        };
        with_duration(&mut worse, 100); // fast
        with_duration(&mut better, 5_000); // slow
        let ranked = rank(&[worse, better]);
        assert_eq!(ranked[0].agent, "better");
    }

    #[test]
    fn preferred_bonus_breaks_near_ties_only() {
        let mut plain = reliable("plain", 20, 20);
        plain.observed_tasks = 20;
        let mut preferred = reliable("preferred", 20, 20);
        preferred.preferred = true;
        let ranked = rank(&[plain.clone(), preferred]);
        assert_eq!(ranked[0].agent, "preferred");

        // A real performance gap is not overridden by the bonus.
        let strong = reliable("strong", 25, 25);
        let mut weak_preferred = reliable("weak", 15, 20);
        weak_preferred.preferred = true;
        let ranked = rank(&[strong, weak_preferred]);
        assert_eq!(ranked[0].agent, "strong");
        let _ = plain;
    }

    #[test]
    fn retry_penalty_moves_retries_elsewhere_on_near_ties() {
        let mut failed_here = reliable("first-choice", 20, 20);
        failed_here.prior_rejection = true;
        let alternative = reliable("second-choice", 20, 20);
        let ranked = rank(&[failed_here, alternative]);
        assert_eq!(ranked[0].agent, "second-choice");
    }

    #[test]
    fn saturated_candidate_loses_to_a_strong_free_alternative() {
        // Near-tie on quality: the 15% capacity component must move a
        // saturated agent behind a free one (spec: a busy historical favorite
        // does not block a strong eligible alternative).
        let mut saturated = reliable("saturated", 20, 20);
        saturated.inflight = 1; // limit 1 -> 0 free slots
        let free = reliable("free", 18, 20);
        let ranked = rank(&[saturated, free]);
        assert_eq!(ranked[0].agent, "free");
        assert_eq!(ranked[0].free_slots, 1);

        // A large quality gap still ranks the saturated agent first on
        // score — the scheduler's reservation loop then skips it because it
        // has no free slots and takes the next-best candidate.
        let mut strong = reliable("strong", 25, 25);
        strong.inflight = 1;
        let weak = reliable("weak", 12, 20);
        let ranked = rank(&[strong, weak]);
        assert_eq!(ranked[0].agent, "strong");
        assert_eq!(ranked[0].free_slots, 0);
        assert_eq!(ranked[1].agent, "weak");
    }

    #[test]
    fn deterministic_tie_break_uses_pool_order_then_name() {
        let a = reliable("zeta", 20, 20);
        let b = reliable("alpha", 20, 20);
        let ranked = rank(&[a, b]);
        assert_eq!(ranked[0].agent, "zeta"); // pool position wins over name
        assert_eq!(ranked[1].agent, "alpha");
    }

    #[test]
    fn exploration_picks_least_observed_unranked_candidate() {
        let known = reliable("known", 20, 20);
        let mut cold_a = facts("cold-a", None);
        cold_a.observed_tasks = 4;
        let mut cold_b = facts("cold-b", None);
        cold_b.observed_tasks = 1;
        let candidates = vec![known.clone(), cold_a, cold_b];
        let ranked = rank(&candidates);

        let pick = exploration_pick(
            &candidates,
            &ranked,
            RoutingMode::Performance,
            true,
            0, // 0 % 5 == 0 -> explore
        );
        assert_eq!(pick.as_deref(), Some("cold-b"));

        // Off-interval dispatches do not explore.
        assert!(
            exploration_pick(&candidates, &ranked, RoutingMode::Performance, true, 1).is_none()
        );
        // Neither does round-robin mode.
        assert!(exploration_pick(&candidates, &ranked, RoutingMode::RoundRobin, true, 0).is_none());
        // Nor disabled exploration.
        assert!(
            exploration_pick(&candidates, &ranked, RoutingMode::Performance, false, 0).is_none()
        );
        let _ = known;
    }

    #[test]
    fn no_exploration_when_nobody_is_ranked() {
        let candidates = vec![facts("a", None), facts("b", None)];
        let ranked = rank(&candidates);
        assert!(
            exploration_pick(&candidates, &ranked, RoutingMode::Performance, true, 0).is_none()
        );
    }

    #[test]
    fn notes_explain_the_evidence_slice() {
        let ranked = rank(&[reliable("good", 12, 12), facts("cold", None)]);
        assert!(ranked[0].note.contains("role+operation"));
        assert!(ranked[0].note.contains("n=12"));
        assert!(ranked[1].note.contains("insufficient data"));
    }
}
