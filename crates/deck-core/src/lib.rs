//! Pure model types, layout engine, protocol messages and framing, key encoding, config
//! parsing and paths. No I/O beyond `std::path`, no threads, no async.

pub mod action;
pub mod agents;
pub mod base64;
pub mod config;
pub mod keys;
pub mod layout;
pub mod model;
pub mod mouse;
pub mod osc7;
pub mod paths;
pub mod protocol;
pub mod state;
pub mod theme;
pub mod wire;
