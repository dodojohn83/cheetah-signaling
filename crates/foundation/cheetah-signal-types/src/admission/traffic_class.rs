//! Traffic classification shared by runtime and application admission control.

use super::priority::Priority;

/// Coarse classification of inbound work used for per-method rate limiting,
/// priority routing and coalescing.
///
/// The set is intentionally small and fixed so it can be used as a bounded
/// metrics label without introducing tenant/device cardinality.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum TrafficClass {
    /// Northbound or reconciler-issued device commands.
    Command,
    /// Catalog / device directory fragments.
    Catalog,
    /// Device keepalive / heartbeat events.
    Keepalive,
    /// Mobile position / GPS updates.
    Position,
    /// Alarm / notification events.
    Alarm,
    /// Recording or media location queries.
    Location,
    /// Any other classified work.
    Other,
}

impl TrafficClass {
    /// All traffic classes, in a stable order.
    pub const ALL: [TrafficClass; 7] = [
        TrafficClass::Command,
        TrafficClass::Catalog,
        TrafficClass::Keepalive,
        TrafficClass::Position,
        TrafficClass::Alarm,
        TrafficClass::Location,
        TrafficClass::Other,
    ];

    /// Returns a stable, bounded string label suitable for metrics.
    pub const fn as_str(self) -> &'static str {
        match self {
            TrafficClass::Command => "command",
            TrafficClass::Catalog => "catalog",
            TrafficClass::Keepalive => "keepalive",
            TrafficClass::Position => "position",
            TrafficClass::Alarm => "alarm",
            TrafficClass::Location => "location",
            TrafficClass::Other => "other",
        }
    }

    /// Returns the scheduling priority for this class.
    pub const fn priority(self) -> Priority {
        match self {
            TrafficClass::Command => Priority::High,
            TrafficClass::Catalog | TrafficClass::Alarm | TrafficClass::Other => Priority::Normal,
            TrafficClass::Keepalive | TrafficClass::Position | TrafficClass::Location => {
                Priority::Low
            }
        }
    }

    /// Whether redundant events of this class can be coalesced while an earlier
    /// event for the same key is still pending.
    pub const fn is_coalescible(self) -> bool {
        matches!(self, TrafficClass::Keepalive | TrafficClass::Position)
    }
}
