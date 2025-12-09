//! Dodeca - A fully incremental static site generator
//!
//! This module exposes internal components for benchmarking and testing.

pub mod cache_bust;
pub mod cas;
pub mod config;
pub mod content_service;
pub mod data;
pub mod db;
pub mod error_pages;
pub mod file_watcher;
pub mod image;
pub mod link_checker;
pub mod plugin_server;
pub mod plugins;
pub mod queries;
pub mod render;
pub mod search;
pub mod serve;
pub mod svg;
pub mod template;
pub mod types;
pub mod url_rewrite;
