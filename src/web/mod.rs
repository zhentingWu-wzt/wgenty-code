//! Web Module - Plugin Marketplace Web Interface
//!
//! This module provides a web server for the plugin marketplace
//! using Axum framework.

pub mod handlers;
pub mod models;
pub mod routes;
pub mod server;
pub mod templates;

pub use models::*;
pub use server::WebServer;
