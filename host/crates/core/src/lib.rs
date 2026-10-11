//! PerritoDeck host core.
//!
//! [`Server`] accepts WebSocket connections from clients, performs the `hello`
//! handshake, keeps a registry of connected devices, answers heartbeats and key
//! presses, and reports everything that happens as [`ServerEvent`]s. It knows
//! nothing about actions or profiles yet; a [`LayoutProvider`] decides what grid
//! each device gets.

mod auth;
pub mod demo;
mod server;

pub use auth::{generate_token, tokens_match};
pub use server::{LayoutProvider, Server, ServerConfig, ServerEvent, ServerHandle};
