#![warn(missing_docs)]

//! Bitfold: a small public API facade for the workspace.
//!
//! This crate provides a clean, stable surface that re-exports
//! the most commonly used types to build reliable UDP apps:
//!
//! - Host and events (`Host`, `SocketEvent`)
//! - Packet types and guarantees (`Packet`, `DeliveryGuarantee`, ...)
//! - Core configuration (`Config`)
//!
//! Example
//! ```ignore
//! use malnet::{Host, SocketEvent, Packet, DeliveryGuarantee, OrderingGuarantee};
//!
//! let mut host = Host::bind_any().unwrap();
//! let remote = host.local_addr().unwrap();
//!
//! // Send a reliable, unordered packet to ourselves
//! let pkt = Packet::reliable_unordered(remote, b"hello".to_vec());
//! host.send(pkt).unwrap();
//!
//! // Poll once
//! use std::time::Instant;
//! host.manual_poll(Instant::now());
//!
//! if let Some(SocketEvent::Packet(rx)) = host.recv() {
//!     assert_eq!(rx.payload(), b"hello");
//! }
//! ```

// Re-export all workspace crates
pub use malnet_core as core;
// Core config
pub use malnet_core::config::{CompressionAlgorithm, Config};
pub use malnet_core::utilities;
pub use malnet_host as host;
// Host: manages multiple peer sessions and events
pub use malnet_host::{Host, SocketEvent};
pub use malnet_peer as peer;
pub use malnet_protocol as protocol;
// Protocol: packets and guarantees
pub use malnet_protocol::{DeliveryGuarantee, OrderingGuarantee, Packet, PacketInfo, PacketType};

/// Convenience prelude with the most commonly used items.
pub mod prelude {
    pub use crate::{
        CompressionAlgorithm, Config, DeliveryGuarantee, Host, OrderingGuarantee, Packet,
        PacketInfo, PacketType, SocketEvent,
    };
}
