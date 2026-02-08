//! Heartbeat agent pattern for proactive scheduled assessment.
//!
//! A heartbeat differs from a plain cron job: it **wakes**, **assesses** whether
//! anything needs attention, and only **acts** if the assessment says so. Between
//! beats the agent can remember context through episodic memory.
//!
//! Example use cases:
//! - HIPAA compliance agent: heartbeats every 30 min, checks policy state vs baseline
//! - SOX transaction monitor: heartbeats every 15 min, queries for anomalies

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Configuration for a heartbeat agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatConfig {
    /// Maximum time the assessment phase may take before timeout.
    pub assessment_timeout: Duration,
    /// Maximum time the action phase may take before timeout.
    pub action_timeout: Duration,
    /// How the heartbeat manages context across beats.
    pub context_mode: HeartbeatContextMode,
    /// After this many consecutive `AllClear` assessments, the scheduler may
    /// adaptively extend the interval between beats (backoff).
    pub consecutive_clear_backoff_threshold: u32,
    /// Maximum backoff multiplier (e.g. 4× means 30 min → 2 h max).
    pub max_backoff_multiplier: u32,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            assessment_timeout: Duration::from_secs(30),
            action_timeout: Duration::from_secs(120),
            context_mode: HeartbeatContextMode::EphemeralWithSummary,
            consecutive_clear_backoff_threshold: 5,
            max_backoff_multiplier: 4,
        }
    }
}

/// How the heartbeat agent manages context across beats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HeartbeatContextMode {
    /// The same `AgentContext` is shared across all beats — full memory persistence.
    SharedPersistent,
    /// Each beat starts with a fresh context, but the summary of the previous beat
    /// is injected as a system message. Balances memory with context continuity.
    #[default]
    EphemeralWithSummary,
    /// Fully ephemeral — no cross-beat context at all.
    FullyEphemeral,
}

/// Severity level for a heartbeat assessment that requires action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeartbeatSeverity {
    /// Informational — log but low urgency.
    Info,
    /// Warning — should be addressed soon.
    Warning,
    /// Critical — requires immediate action.
    Critical,
}

/// Result of the heartbeat's assessment phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HeartbeatAssessment {
    /// Something needs attention.
    NeedsAction {
        reason: String,
        severity: HeartbeatSeverity,
        data: Option<serde_json::Value>,
    },
    /// Everything looks good — no action needed.
    AllClear { summary: String },
    /// The assessment itself failed.
    Error { message: String },
}

/// Tracks the state of a heartbeat across beats for adaptive scheduling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatState {
    /// How many consecutive `AllClear` assessments have occurred.
    pub consecutive_clear_count: u32,
    /// The current backoff multiplier (1 = normal interval).
    pub current_backoff: u32,
    /// Last assessment result summary.
    pub last_assessment_summary: Option<String>,
    /// Timestamp of the last beat.
    pub last_beat_at: Option<DateTime<Utc>>,
    /// Total number of beats executed.
    pub total_beats: u64,
    /// Total number of actions taken.
    pub total_actions: u64,
}

impl Default for HeartbeatState {
    fn default() -> Self {
        Self {
            consecutive_clear_count: 0,
            current_backoff: 1,
            last_assessment_summary: None,
            last_beat_at: None,
            total_beats: 0,
            total_actions: 0,
        }
    }
}

impl HeartbeatState {
    /// Update state after an assessment. Returns the new backoff multiplier.
    pub fn record_assessment(
        &mut self,
        assessment: &HeartbeatAssessment,
        config: &HeartbeatConfig,
    ) -> u32 {
        self.total_beats += 1;
        self.last_beat_at = Some(Utc::now());

        match assessment {
            HeartbeatAssessment::AllClear { summary } => {
                self.consecutive_clear_count += 1;
                self.last_assessment_summary = Some(summary.clone());

                // Adaptive backoff: if enough consecutive clears, increase interval.
                if self.consecutive_clear_count >= config.consecutive_clear_backoff_threshold {
                    self.current_backoff =
                        (self.current_backoff * 2).min(config.max_backoff_multiplier);
                }
            }
            HeartbeatAssessment::NeedsAction { reason, .. } => {
                self.consecutive_clear_count = 0;
                self.current_backoff = 1; // Reset to normal interval.
                self.total_actions += 1;
                self.last_assessment_summary = Some(reason.clone());
            }
            HeartbeatAssessment::Error { message } => {
                self.consecutive_clear_count = 0;
                // Don't reset backoff on error — could be transient.
                self.last_assessment_summary = Some(format!("ERROR: {}", message));
            }
        }

        self.current_backoff
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_heartbeat_config() {
        let config = HeartbeatConfig::default();
        assert_eq!(config.assessment_timeout, Duration::from_secs(30));
        assert_eq!(config.action_timeout, Duration::from_secs(120));
        assert_eq!(
            config.context_mode,
            HeartbeatContextMode::EphemeralWithSummary
        );
        assert_eq!(config.consecutive_clear_backoff_threshold, 5);
        assert_eq!(config.max_backoff_multiplier, 4);
    }

    #[test]
    fn heartbeat_state_tracks_clears() {
        let config = HeartbeatConfig {
            consecutive_clear_backoff_threshold: 3,
            max_backoff_multiplier: 4,
            ..Default::default()
        };
        let mut state = HeartbeatState::default();

        for _ in 0..3 {
            state.record_assessment(
                &HeartbeatAssessment::AllClear {
                    summary: "ok".to_string(),
                },
                &config,
            );
        }
        assert_eq!(state.consecutive_clear_count, 3);
        assert_eq!(state.current_backoff, 2);
        assert_eq!(state.total_beats, 3);
        assert_eq!(state.total_actions, 0);
    }

    #[test]
    fn heartbeat_state_resets_on_action() {
        let config = HeartbeatConfig::default();
        let mut state = HeartbeatState::default();
        state.consecutive_clear_count = 10;
        state.current_backoff = 4;

        state.record_assessment(
            &HeartbeatAssessment::NeedsAction {
                reason: "drift detected".to_string(),
                severity: HeartbeatSeverity::Warning,
                data: None,
            },
            &config,
        );
        assert_eq!(state.consecutive_clear_count, 0);
        assert_eq!(state.current_backoff, 1);
        assert_eq!(state.total_actions, 1);
    }

    #[test]
    fn backoff_caps_at_max() {
        let config = HeartbeatConfig {
            consecutive_clear_backoff_threshold: 1,
            max_backoff_multiplier: 4,
            ..Default::default()
        };
        let mut state = HeartbeatState::default();

        // Keep recording all-clear — backoff should cap at 4.
        for _ in 0..20 {
            state.record_assessment(
                &HeartbeatAssessment::AllClear {
                    summary: "ok".to_string(),
                },
                &config,
            );
        }
        assert_eq!(state.current_backoff, 4);
    }

    #[test]
    fn error_does_not_reset_backoff() {
        let config = HeartbeatConfig::default();
        let mut state = HeartbeatState::default();
        state.current_backoff = 3;

        state.record_assessment(
            &HeartbeatAssessment::Error {
                message: "timeout".to_string(),
            },
            &config,
        );
        assert_eq!(state.current_backoff, 3); // Unchanged.
        assert_eq!(state.consecutive_clear_count, 0);
    }

    #[test]
    fn heartbeat_assessment_serialization() {
        let assessment = HeartbeatAssessment::NeedsAction {
            reason: "policy drift".to_string(),
            severity: HeartbeatSeverity::Critical,
            data: Some(serde_json::json!({"drift_count": 3})),
        };
        let json = serde_json::to_string(&assessment).unwrap();
        let deserialized: HeartbeatAssessment = serde_json::from_str(&json).unwrap();
        match deserialized {
            HeartbeatAssessment::NeedsAction {
                reason, severity, ..
            } => {
                assert_eq!(reason, "policy drift");
                assert_eq!(severity, HeartbeatSeverity::Critical);
            }
            _ => panic!("expected NeedsAction"),
        }
    }
}
