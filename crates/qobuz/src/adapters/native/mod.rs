//! Native in-process Qobuz ripper adapter and scraping utilities.

pub mod bundle;
pub mod client;
pub mod signature;

pub use client::{NativeConfig, NativeRipperAdapter};
