//! Generated tonic gRPC service bindings for Cheetah Signaling.
//!
//! Message types live in `cheetah-signal-contracts`; this crate only provides
//! the tonic `Client`/`Server` traits for packages that define services.

#![warn(missing_docs)]

/// Cheetah package namespace.
pub mod cheetah {
    /// Foundation types shared across all domains.
    pub mod foundation {
        /// Version 1 foundation types.
        pub mod v1 {
            pub use cheetah_signal_contracts::cheetah::foundation::v1::*;
        }
    }

    /// Common shared types and services.
    pub mod common {
        /// Version 1 common types and services.
        #[allow(
            missing_docs,
            missing_debug_implementations,
            clippy::large_enum_variant
        )]
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/cheetah.common.v1.rs"));
            pub use cheetah_signal_contracts::cheetah::common::v1::*;
        }
    }

    /// Device domain messages.
    pub mod device {
        /// Version 1 device domain messages.
        pub mod v1 {
            pub use cheetah_signal_contracts::cheetah::device::v1::*;
        }
    }

    /// Control domain messages.
    pub mod control {
        /// Version 1 control domain messages.
        pub mod v1 {
            pub use cheetah_signal_contracts::cheetah::control::v1::*;
        }
    }

    /// Media domain messages and services.
    pub mod media {
        /// Version 1 media domain messages and services.
        #[allow(
            missing_docs,
            missing_debug_implementations,
            clippy::large_enum_variant
        )]
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/cheetah.media.v1.rs"));
            pub use cheetah_signal_contracts::cheetah::media::v1::*;
        }
    }

    /// Plugin domain messages and services.
    pub mod plugin {
        /// Version 1 plugin domain messages and services.
        #[allow(
            missing_docs,
            missing_debug_implementations,
            clippy::large_enum_variant
        )]
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/cheetah.plugin.v1.rs"));
            pub use cheetah_signal_contracts::cheetah::plugin::v1::*;
        }
    }

    /// Cluster domain messages.
    pub mod cluster {
        /// Version 1 cluster domain messages.
        pub mod v1 {
            pub use cheetah_signal_contracts::cheetah::cluster::v1::*;
        }
    }
}
