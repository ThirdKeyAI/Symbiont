//! Per-Step Iteration Cap (Progress Tracker)
//!
//! Higher-order concern for coordinators. Tracks per-step reattempt counts
//! and detects stuck loops via normalized Levenshtein similarity of
//! consecutive error outputs. Part of the orga-adaptive feature gate.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// What to do when a step hits its reattempt limit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LimitAction {
    /// Skip the step and move on.
    SkipStep,
    /// Abort the entire task.
    AbortTask,
    /// Escalate to a human or external system.
    Escalate,
}

impl Default for LimitAction {
    fn default() -> Self {
        Self::SkipStep
    }
}

/// Configuration for per-step iteration limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepIterationConfig {
    /// Maximum reattempts per step before triggering the limit action.
    #[serde(default = "default_max_reattempts")]
    pub max_reattempts_per_step: u32,
    /// Similarity threshold (0.0–1.0) for detecting stuck loops.
    /// If consecutive error outputs have similarity >= this threshold,
    /// the step is considered stuck.
    #[serde(default = "default_similarity_threshold")]
    pub similarity_threshold: f64,
    /// Action to take when the reattempt limit is reached.
    #[serde(default)]
    pub on_limit_reached: LimitAction,
}

fn default_max_reattempts() -> u32 {
    2
}

fn default_similarity_threshold() -> f64 {
    0.85
}

impl Default for StepIterationConfig {
    fn default() -> Self {
        Self {
            max_reattempts_per_step: default_max_reattempts(),
            similarity_threshold: default_similarity_threshold(),
            on_limit_reached: LimitAction::default(),
        }
    }
}

/// The decision for whether a step should continue.
#[derive(Debug, Clone)]
pub enum StepDecision {
    /// The step may continue.
    Continue,
    /// The step should stop.
    Stop { reason: String },
}

/// Internal tracking state for a single step.
#[derive(Debug)]
struct StepProgress {
    attempts: u32,
    last_error: Option<String>,
}

/// Tracks per-step reattempt counts and detects stuck loops.
///
/// Not wired into the reasoning loop directly — coordinators call
/// `should_continue()` between steps and emit `LoopEvent::StepLimitReached`.
pub struct ProgressTracker {
    config: StepIterationConfig,
    steps: HashMap<String, StepProgress>,
}

impl ProgressTracker {
    /// Create a new tracker with the given configuration.
    pub fn new(config: StepIterationConfig) -> Self {
        Self {
            config,
            steps: HashMap::new(),
        }
    }

    /// Begin tracking a step. Resets the step if it was previously tracked.
    pub fn begin_step(&mut self, step_id: impl Into<String>) {
        let id = step_id.into();
        self.steps.insert(
            id,
            StepProgress {
                attempts: 0,
                last_error: None,
            },
        );
    }

    /// Record an attempt for a step with an error output.
    pub fn record_attempt(&mut self, step_id: &str, error_output: impl Into<String>) {
        let output = error_output.into();
        if let Some(progress) = self.steps.get_mut(step_id) {
            progress.attempts += 1;
            progress.last_error = Some(output);
        }
    }

    /// Determine whether a step should continue or stop.
    pub fn should_continue(&self, step_id: &str) -> StepDecision {
        let progress = match self.steps.get(step_id) {
            Some(p) => p,
            None => return StepDecision::Continue,
        };

        // Check max reattempts
        if progress.attempts >= self.config.max_reattempts_per_step {
            return StepDecision::Stop {
                reason: format!(
                    "Step '{}' reached max reattempts ({})",
                    step_id, self.config.max_reattempts_per_step
                ),
            };
        }

        StepDecision::Continue
    }

    /// Record an attempt and check if similar to the previous error (stuck detection).
    pub fn record_and_check(
        &mut self,
        step_id: &str,
        error_output: impl Into<String>,
    ) -> StepDecision {
        let output = error_output.into();

        if let Some(progress) = self.steps.get(step_id) {
            // Check similarity with previous error before recording
            if let Some(ref last) = progress.last_error {
                let similarity = normalized_levenshtein(last, &output);
                if similarity >= self.config.similarity_threshold {
                    // Record and stop — similar errors indicate a stuck loop
                    let attempts = progress.attempts + 1;
                    if let Some(p) = self.steps.get_mut(step_id) {
                        p.attempts = attempts;
                        p.last_error = Some(output);
                    }
                    return StepDecision::Stop {
                        reason: format!(
                            "Step '{}' appears stuck: consecutive errors are {:.0}% similar (threshold: {:.0}%)",
                            step_id,
                            similarity * 100.0,
                            self.config.similarity_threshold * 100.0
                        ),
                    };
                }
            }
        }

        self.record_attempt(step_id, &output);
        self.should_continue(step_id)
    }

    /// Get the current attempt count for a step.
    pub fn attempt_count(&self, step_id: &str) -> u32 {
        self.steps.get(step_id).map_or(0, |p| p.attempts)
    }

    /// Reset tracking for a step, removing all recorded state.
    pub fn reset_step(&mut self, step_id: &str) {
        self.steps.remove(step_id);
    }

    /// Get the configured limit action.
    pub fn limit_action(&self) -> &LimitAction {
        &self.config.on_limit_reached
    }
}

/// Compute the normalized Levenshtein distance between two strings.
/// Returns a value in [0.0, 1.0] where 1.0 means identical.
/// Uses O(min(m, n)) space.
fn normalized_levenshtein(a: &str, b: &str) -> f64 {
    let a_len = a.chars().count();
    let b_len = b.chars().count();

    if a_len == 0 && b_len == 0 {
        return 1.0;
    }
    if a_len == 0 || b_len == 0 {
        return 0.0;
    }

    // Ensure we iterate over the longer string in the outer loop
    // and keep the shorter one in the DP row for O(min(m,n)) space
    let (short, long, short_len, long_len) = if a_len <= b_len {
        (a, b, a_len, b_len)
    } else {
        (b, a, b_len, a_len)
    };

    let mut prev_row: Vec<usize> = (0..=short_len).collect();
    let mut curr_row = vec![0usize; short_len + 1];

    for (i, long_ch) in long.chars().enumerate() {
        curr_row[0] = i + 1;
        for (j, short_ch) in short.chars().enumerate() {
            let cost = if long_ch == short_ch { 0 } else { 1 };
            curr_row[j + 1] = (prev_row[j] + cost)
                .min(prev_row[j + 1] + 1)
                .min(curr_row[j] + 1);
        }
        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    let distance = prev_row[short_len];
    let max_len = long_len;
    1.0 - (distance as f64 / max_len as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_identical() {
        assert!((normalized_levenshtein("hello", "hello") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_levenshtein_empty_both() {
        assert!((normalized_levenshtein("", "") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_levenshtein_one_empty() {
        assert!((normalized_levenshtein("", "hello") - 0.0).abs() < f64::EPSILON);
        assert!((normalized_levenshtein("hello", "") - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_levenshtein_similar() {
        // "kitten" vs "sitting": edit distance 3, max len 7
        // similarity = 1 - 3/7 ≈ 0.571
        let sim = normalized_levenshtein("kitten", "sitting");
        assert!(sim > 0.5 && sim < 0.6);
    }

    #[test]
    fn test_levenshtein_completely_different() {
        let sim = normalized_levenshtein("abc", "xyz");
        assert!(sim < 0.1);
    }

    #[test]
    fn test_continue_on_first_attempt() {
        let mut tracker = ProgressTracker::new(StepIterationConfig::default());
        tracker.begin_step("step1");
        assert!(matches!(
            tracker.should_continue("step1"),
            StepDecision::Continue
        ));
    }

    #[test]
    fn test_stop_at_max_attempts() {
        let mut tracker = ProgressTracker::new(StepIterationConfig {
            max_reattempts_per_step: 2,
            ..Default::default()
        });
        tracker.begin_step("step1");
        tracker.record_attempt("step1", "error A");
        assert!(matches!(
            tracker.should_continue("step1"),
            StepDecision::Continue
        ));
        tracker.record_attempt("step1", "error B");
        assert!(matches!(
            tracker.should_continue("step1"),
            StepDecision::Stop { .. }
        ));
    }

    #[test]
    fn test_stop_on_similar_errors() {
        let mut tracker = ProgressTracker::new(StepIterationConfig {
            max_reattempts_per_step: 10, // high limit to test similarity
            similarity_threshold: 0.85,
            ..Default::default()
        });
        tracker.begin_step("step1");
        tracker.record_attempt("step1", "connection timeout to api.example.com:443");

        let decision =
            tracker.record_and_check("step1", "connection timeout to api.example.com:443");
        assert!(matches!(decision, StepDecision::Stop { .. }));
    }

    #[test]
    fn test_continue_on_different_errors() {
        let mut tracker = ProgressTracker::new(StepIterationConfig {
            max_reattempts_per_step: 10,
            similarity_threshold: 0.85,
            ..Default::default()
        });
        tracker.begin_step("step1");
        tracker.record_attempt("step1", "connection timeout to api.example.com");

        let decision = tracker.record_and_check("step1", "permission denied for /etc/secret");
        assert!(matches!(decision, StepDecision::Continue));
    }

    #[test]
    fn test_begin_step_resets() {
        let mut tracker = ProgressTracker::new(StepIterationConfig::default());
        tracker.begin_step("step1");
        tracker.record_attempt("step1", "error");
        assert_eq!(tracker.attempt_count("step1"), 1);

        tracker.begin_step("step1");
        assert_eq!(tracker.attempt_count("step1"), 0);
    }

    #[test]
    fn test_reset_step_removes() {
        let mut tracker = ProgressTracker::new(StepIterationConfig::default());
        tracker.begin_step("step1");
        tracker.record_attempt("step1", "error");
        tracker.reset_step("step1");
        assert_eq!(tracker.attempt_count("step1"), 0);
    }

    #[test]
    fn test_default_config_values() {
        let config = StepIterationConfig::default();
        assert_eq!(config.max_reattempts_per_step, 2);
        assert!((config.similarity_threshold - 0.85).abs() < f64::EPSILON);
        assert!(matches!(config.on_limit_reached, LimitAction::SkipStep));
    }
}
