//! A playable title as the streaming layer sees it: its file (`MediaSource`) and that file's
//! tracks (`MediaStream`), numbered and labelled by `library::streams`. Made from the library
//! for each playback request.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Item {
    pub id: String,
    pub name: Option<String>,
    /// "Movie", "Episode"…
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub provider_ids: HashMap<String, String>,
    pub series_id: Option<String>,
    pub series_name: Option<String>,
    /// Season number (episodes).
    pub parent_index_number: Option<i64>,
    /// Episode number (episodes).
    pub index_number: Option<i64>,
    #[serde(default)]
    pub media_sources: Vec<MediaSource>,
    /// The series' provider ids.
    #[serde(skip)]
    pub series_provider_ids: HashMap<String, String>,
}

impl Item {
    /// A provider id ("Imdb", "Tmdb"), case-insensitively.
    pub fn provider(&self, name: &str) -> Option<&str> {
        self.provider_ids
            .iter()
            .find(|(k, v)| k.eq_ignore_ascii_case(name) && !v.is_empty())
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaSource {
    pub id: String,
    pub path: Option<String>,
    pub run_time_ticks: Option<i64>,
    pub bitrate: Option<i64>,
    #[serde(default)]
    pub media_streams: Vec<MediaStream>,
    /// The file's size and mtime (ns) as the library last saw them: the probe cache's key
    /// without asking the share again.
    #[serde(skip)]
    pub stamp: Option<(u64, i64)>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaStream {
    pub index: i64,
    #[serde(rename = "Type")]
    pub kind: String,
    pub codec: Option<String>,
    pub language: Option<String>,
    pub title: Option<String>,
    pub display_title: Option<String>,
    pub channels: Option<i64>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub is_forced: bool,
    #[serde(default)]
    pub is_external: bool,
    #[serde(default)]
    pub is_text_subtitle_stream: bool,
    pub video_range_type: Option<String>,
    /// The sidecar file of an external subtitle stream.
    pub path: Option<String>,
}
