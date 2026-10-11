//! PerritoDeck host core.
//!
//! [`Server`] accepts WebSocket connections from clients (over TLS with pairing by
//! default), performs the `hello` / `pair` handshake, keeps a registry of connected
//! devices, answers heartbeats and key presses, and reports everything as
//! [`ServerEvent`]s. A [`LayoutProvider`] decides what grid each device gets.

pub mod demo;
pub mod discovery;
pub mod identity;
pub mod pairing;
mod server;

pub use identity::{display_fingerprint, HostIdentity};
pub use pairing::PairingStore;
pub use server::{LayoutProvider, Security, Server, ServerConfig, ServerEvent, ServerHandle};
