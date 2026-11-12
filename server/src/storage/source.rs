//! Where a title's file is read from: a local path (a folder or a mount in the container) or an
//! SMB share the server reads itself (`smb://…`, see `vfs.rs` and `smb.rs`).

use std::path::PathBuf;

use anyhow::Result;
use bytes::Bytes;

use crate::storage::vfs;

#[derive(Clone, Debug)]
pub enum Source {
    Local(PathBuf),
    /// `smb://host/share/path`.
    Smb(String),
}

impl Source {
    /// The source of a library path.
    pub fn of(path: &str) -> Source {
        if vfs::is_smb(path) {
            Source::Smb(path.to_string())
        } else {
            Source::Local(path.into())
        }
    }

    /// The path as the library keeps it.
    pub fn path(&self) -> String {
        match self {
            Source::Local(p) => p.to_string_lossy().into_owned(),
            Source::Smb(u) => u.clone(),
        }
    }

    /// ffmpeg/ffprobe arguments that open this source (`-i path`; a share through the loopback).
    pub fn input_args(&self) -> Vec<String> {
        vfs::ffmpeg_input_args(&self.path())
    }

    /// `input_args`, and for a share a count of the bytes the run has read (see `vfs::ReadCount`).
    pub fn input_args_counted(&self) -> (Vec<String>, Option<vfs::ReadCount>) {
        vfs::ffmpeg_input_args_counted(&self.path())
    }

    pub fn describe(&self) -> &'static str {
        match self {
            Source::Local(_) => "local",
            Source::Smb(_) => "smb",
        }
    }

    /// `len` bytes at `offset` (fewer at the end of the file).
    pub async fn read_at(&self, _http: &reqwest::Client, offset: u64, len: u64) -> Result<Bytes> {
        vfs::read_at(&self.path(), offset, len).await
    }
}
