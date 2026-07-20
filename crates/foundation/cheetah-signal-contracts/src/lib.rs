//! Generated Protocol Buffer and gRPC bindings for Cheetah Signaling.
//!
//! This crate is generated from `proto/` and must not be edited by hand.

/// Cheetah package namespace.
pub mod cheetah {
    /// Foundation types shared across all domains.
    pub mod foundation {
        /// Version 1 foundation types.
        #[allow(
            missing_docs,
            missing_debug_implementations,
            clippy::large_enum_variant
        )]
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/cheetah.foundation.v1.rs"));
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
        }
    }

    /// Device domain messages.
    pub mod device {
        /// Version 1 device domain messages.
        #[allow(
            missing_docs,
            missing_debug_implementations,
            clippy::large_enum_variant
        )]
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/cheetah.device.v1.rs"));
        }
    }

    /// Control domain messages.
    pub mod control {
        /// Version 1 control domain messages.
        #[allow(
            missing_docs,
            missing_debug_implementations,
            clippy::large_enum_variant
        )]
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/cheetah.control.v1.rs"));
        }
    }

    /// Media domain messages.
    pub mod media {
        /// Version 1 media domain messages.
        #[allow(
            missing_docs,
            missing_debug_implementations,
            clippy::large_enum_variant
        )]
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/cheetah.media.v1.rs"));
        }
    }

    /// Plugin runtime messages.
    pub mod plugin {
        /// Version 1 plugin runtime messages.
        #[allow(
            missing_docs,
            missing_debug_implementations,
            clippy::large_enum_variant
        )]
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/cheetah.plugin.v1.rs"));
        }
    }

    /// Cluster membership messages.
    pub mod cluster {
        /// Version 1 cluster membership messages.
        #[allow(
            missing_docs,
            missing_debug_implementations,
            clippy::large_enum_variant
        )]
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/cheetah.cluster.v1.rs"));
        }
    }
}

/// Contract versioning and compatibility constants.
pub mod version;
