//! SIP transaction identifier and Sans-I/O state machine.

mod client;
mod key;
mod manager;
mod server;
mod state_machine;
mod timers;

pub use key::{BranchPolicy, TransactionHalf, TransactionKey, TransactionKind, TransactionRole};
pub use manager::{
    DEFAULT_MAX_TRANSACTIONS, DEFAULT_TRANSACTION_TTL, ManagerConfig, ManagerOutput,
    RequestOutcome, TransactionManager,
};
pub use state_machine::{
    Transaction, TransactionConfig, TransactionEvent, TransactionOutput, TransportKind,
};
pub use timers::{TimerKind, TimerSet};
