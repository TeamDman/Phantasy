
use serde::Deserialize;
use serde::Serialize;

use crate::uri::Uri;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackAudioFeatures {
    pub acousticness: f64,
    #[serde(rename = "analysis_url")]
    pub analysis_url: Uri,
    pub danceability: f64,
    #[serde(rename = "duration_ms")]
    pub duration_ms: i64,
    pub energy: f64,
    pub id: String,
    pub instrumentalness: f64,
    pub key: i64,
    pub liveness: f64,
    pub loudness: f64,
    pub mode: i64,
    pub speechiness: f64,
    pub tempo: f64,
    #[serde(rename = "time_signature")]
    pub time_signature: i64,
    #[serde(rename = "track_href")]
    pub track_href: Uri,
    #[serde(rename = "type")]
    pub type_field: String,
    pub uri: String,
    pub valence: f64,
}
