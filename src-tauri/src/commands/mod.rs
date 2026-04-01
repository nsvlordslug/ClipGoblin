//! Command modules — each file groups related Tauri commands.
//!
//! Re-exports all command functions so `lib.rs` can reference them
//! directly in `tauri::generate_handler![]`.

pub mod auth;
pub mod captions;
pub mod clip;
pub mod export;
pub mod scheduled;
pub mod settings;
pub mod social;
pub mod vod;
