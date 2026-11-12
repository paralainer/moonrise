//! Moonrise server: streams for the Moonrise app (instant HLS cut at the file's keyframes, every audio
//! track as a native rendition, converted when Apple devices can't play it, WebVTT subtitles,
//! GPU transcodes, downloads, subtitle search) and, from Phase 2, its own library: the media
//! shares scanned, titles identified (TMDB, an LLM for the odd ones),
//! TMDB metadata and artwork, search, a read-only browse API.
//!
//! The crate is a library with a thin binary (`main.rs`). Its modules, by layer:
//!
//! - `api`, `auth`, `userdata`, `downloads`, `offline`: the HTTP API and what it keeps per user;
//! - `library`: scanning, identification, metadata, browse and search;
//! - `media`: one title served as HLS (keyframe indexes, cutting, transcoding, audio, trickplay,
//!   credits);
//! - `subtitles`: text subtitles, OpenSubtitles and timing;
//! - `llm`: the LLM step of identification and its providers;
//! - `storage`: local folders and SMB shares, and the segment cache; `db`: the database;
//! - `admin`: configuration, settings, the admin and manage APIs, claiming, the log;
//! - `cli`, `mdns`, `migrate`: the command line, Bonjour, moving a library root.

mod admin;
mod api;
mod auth;
mod cli;
mod db;
mod downloads;
#[cfg(test)]
mod golden;
mod library;
mod llm;
mod mdns;
mod media;
mod migrate;
mod offline;
mod storage;
mod subtitles;
mod userdata;

pub use cli::run;
