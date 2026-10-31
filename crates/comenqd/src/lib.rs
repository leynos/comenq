#![cfg_attr(docsrs, feature(doc_cfg))]

//! Library components for the Comenqd daemon.
//!
//! # Overview
//! This crate exposes:
//! - [`config::Config`] — typed, validated daemon configuration loaded from
//!   `/etc/comenqd/config.toml` with environment and CLI overrides.
//! - [`metrics`] — bounded Prometheus metrics for tasks, requests, cooldowns,
//!   and GitHub posting.
//! - [`queue::SharedQueue`] and [`store::QueueStore`] — persistent queue
//!   scheduling and reorderable comment storage.
//! - [`daemon`] — the public facade for the listener, worker, and supervisor.
//!
//! # Examples
//! ```rust,no_run
//! use comenqd::config::Config;
//!
//! let cfg = Config::load().expect("configuration must be valid");
//! println!("socket: {}", cfg.socket_path.display());
//! ```
pub mod config;
pub mod metrics;

mod listener;
pub mod queue;
pub mod store;
mod supervisor;
mod worker;

pub mod daemon;
