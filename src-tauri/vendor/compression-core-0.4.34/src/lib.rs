//! Abstractions for compression algorithms.

#![cfg_attr(not(feature = "legacy-output"), forbid(unsafe_code))]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod level;
#[cfg(feature = "legacy-output")]
pub mod unshared;
pub mod util;

pub use level::Level;
