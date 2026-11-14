//! Talking to the hardware without anything in between: KMS (screen),
//! evdev + HDMI-CEC (remote), ALSA (sound), V4L2 (H.264 decoder).
#[cfg(target_os = "linux")]
pub mod alsa;
#[cfg(target_os = "linux")]
pub mod cec;
pub mod input;
#[cfg(target_os = "linux")]
pub mod kms;
pub mod system;
#[cfg(target_os = "linux")]
pub mod uinput;
