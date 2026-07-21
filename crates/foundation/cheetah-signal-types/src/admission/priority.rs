//! Traffic priority levels used by admission and scheduling.

/// Relative priority of a unit of work.
///
/// Ordering is defined so that `Low < Normal < High`, which lets callers use
/// standard comparison operators to decide which work to shed first under
/// overload.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Priority {
    /// Best-effort work that may be shed first under overload
    /// (for example keepalive or position updates).
    Low,
    /// Default priority for informational events such as catalog fragments.
    Normal,
    /// Latency- and correctness-sensitive work such as commands and ownership
    /// changes that must not be shed.
    High,
}

impl Priority {
    /// Returns a stable, bounded string label suitable for metrics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
        }
    }
}
