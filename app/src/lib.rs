//! d2rs app library: the GUI and the command line tools; `main.rs` is a thin binary around it.
pub mod cli;
pub mod create;
pub mod gui;
pub mod items;
#[cfg(not(target_arch = "wasm32"))]
pub mod live;
pub mod map;
pub mod platform;
pub mod setup_ui;
pub mod trait_ed;
