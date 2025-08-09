#![cfg_attr(docsrs, feature(doc_cfg))]

//! Library components for the Comenqd daemon.
//!
//! # Overview
//! This crate exposes:
//! - [`config::Config`] — typed, validated daemon configuration loaded from
//!   `/etc/comenqd/config.toml` with environment and CLI overrides.
//! - Further daemon-specific helpers (to be added).
//!
//! # Examples
//! ```rust,no_run
//! use comenqd::config::Config;
//!
//! let cfg = Config::load().expect("configuration must be valid");
//! println!("socket: {}", cfg.socket_path.display());
//! ```
pub mod config;
pub mod daemon;
