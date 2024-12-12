//! Cross Platform, Performant and High Quality screen recordings

pub mod capturer;
pub mod frame;
mod targets;
mod utils;

// Helper Methods
pub use targets::*;
pub use utils::has_permission;
pub use utils::is_supported;
pub use utils::request_permission;

#[cfg(target_os = "macos")]
pub mod engine {
    pub use crate::capturer::engine::mac;
}
