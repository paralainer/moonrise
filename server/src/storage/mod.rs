//! Where media is read from: a title's source (a local path or an SMB share), file access over
//! both (`vfs`), the SMB client itself, and the disk cache for cut segments.

pub mod cache;
pub mod smb;
#[cfg(test)]
mod smb_it;
pub mod source;
pub mod vfs;
