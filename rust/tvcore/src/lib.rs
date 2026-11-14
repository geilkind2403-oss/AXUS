//! Shared by `tv-server` (runs on the PC) and `tv-launcher` (runs on the Pi):
//! the film library schema, the music library scanner (ID3 tags, durations,
//! covers, lyrics) and the small pure helpers (track picking, WebVTT shifting).
pub mod films;
pub mod music;
pub mod tracks;
pub mod util;

pub use util::id_for;
