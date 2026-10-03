//! Audio capture: `linux` (parec) et `windows` (WASAPI micro + loopback).
//!
//! Les autres modules utilisent `crate::audio::{Source, to_meter, ...}` :
//! la réexportation ci-dessous garde ces usages compatibles sur les deux OS.
//!
//! `RATE`, `CHANNELS` et `HISTORY` sont définis une seule fois ici ; les
//! deux backends (`linux` parec et `windows` WASAPI) s'y réfèrent
//! (`super::RATE`, ...) afin que tout chunk capturé reste en s16le 48 kHz
//! stéréo comme l'attend `transcribe.rs`.

/// 48 kHz, imposé par `transcribe.rs` (downsample vers 16 kHz).
pub const RATE: u32 = 48_000;
/// Stéréo : tout chunk capturé est converti vers ce format.
pub const CHANNELS: u32 = 2;
/// Three seconds of 20 ms peaks.
pub const HISTORY: usize = 150;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "linux")]
pub use self::linux::*;
#[cfg(target_os = "windows")]
pub use self::windows::*;
