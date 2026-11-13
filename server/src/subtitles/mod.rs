//! Subtitles: text tracks read from the file (`subx`), served as SRT and WebVTT (`subs`), found on
//! OpenSubtitles.com, and matched to the file's timing (`subsync`).

pub mod bitmap;
pub mod opensubtitles;
pub mod pictures;
pub mod subs;
pub mod subsync;
pub mod subx;
