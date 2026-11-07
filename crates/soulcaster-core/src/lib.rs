//! Soulcaster's engine vocabulary, shared by the app, the CLI and the engine. It has no GUI
//! dependency and starts no process: anything external goes through [`command::CommandRunner`].

pub mod ask;
pub mod catalog;
pub mod command;
pub mod config;
pub mod event;
pub mod lifecycle;
pub mod lint;
pub mod plan;
pub mod recipe;
