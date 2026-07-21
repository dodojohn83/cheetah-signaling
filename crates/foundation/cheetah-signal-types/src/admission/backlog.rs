//! Hysteresis-based backlog / overload state machine.

/// Overload state derived from observed backlog depth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BacklogState {
    /// Operating normally.
    Steady,
    /// Backlog has crossed the high watermark and low-priority work should be
    /// shed until it recovers below the low watermark.
    Overloaded,
}

impl BacklogState {
    /// Returns a stable, bounded string label suitable for metrics.
    pub const fn as_str(self) -> &'static str {
        match self {
            BacklogState::Steady => "steady",
            BacklogState::Overloaded => "overloaded",
        }
    }
}

/// Result of observing a backlog depth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BacklogObservation {
    /// The state after the observation.
    pub state: BacklogState,
    /// `true` if this observation transitioned into overload.
    pub entered_overload: bool,
    /// `true` if this observation transitioned back to steady (recovered).
    pub recovered: bool,
}

/// Tracks overload using two watermarks to provide hysteresis: it enters
/// overload at or above `high_watermark` and only recovers at or below
/// `low_watermark`. This prevents oscillation around a single threshold.
#[derive(Clone, Copy, Debug)]
pub struct BacklogController {
    state: BacklogState,
    high_watermark: u64,
    low_watermark: u64,
    overload_transitions: u64,
    recovery_transitions: u64,
}

impl BacklogController {
    /// Creates a controller. `low_watermark` is clamped to be no greater than
    /// `high_watermark`.
    pub fn new(high_watermark: u64, low_watermark: u64) -> Self {
        Self {
            state: BacklogState::Steady,
            high_watermark: high_watermark.max(1),
            low_watermark: low_watermark.min(high_watermark),
            overload_transitions: 0,
            recovery_transitions: 0,
        }
    }

    /// Observes the current backlog `depth` and updates the state.
    pub fn observe(&mut self, depth: u64) -> BacklogObservation {
        match self.state {
            BacklogState::Steady if depth >= self.high_watermark => {
                self.state = BacklogState::Overloaded;
                self.overload_transitions += 1;
                BacklogObservation {
                    state: self.state,
                    entered_overload: true,
                    recovered: false,
                }
            }
            BacklogState::Overloaded if depth <= self.low_watermark => {
                self.state = BacklogState::Steady;
                self.recovery_transitions += 1;
                BacklogObservation {
                    state: self.state,
                    entered_overload: false,
                    recovered: true,
                }
            }
            _ => BacklogObservation {
                state: self.state,
                entered_overload: false,
                recovered: false,
            },
        }
    }

    /// Returns the current state.
    pub const fn state(&self) -> BacklogState {
        self.state
    }

    /// Whether low-priority work should currently be shed.
    pub const fn shed_low_priority(&self) -> bool {
        matches!(self.state, BacklogState::Overloaded)
    }

    /// Total transitions into overload.
    pub const fn overload_transitions(&self) -> u64 {
        self.overload_transitions
    }

    /// Total transitions back to steady.
    pub const fn recovery_transitions(&self) -> u64 {
        self.recovery_transitions
    }
}
