//! Shared by `tv-server` (runs on the PC) and `tv-launcher` (runs on the Pi):
//! the film library schema and the small pure helpers (track picking, WebVTT
//! shifting, durations).
pub mod films;
pub mod tracks;
pub mod util;

pub use util::id_for;
