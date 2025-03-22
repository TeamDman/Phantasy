use crate::bearer_token::BearerToken;
use crate::fetch::fetch;
use crate::track_audio_features::TrackAudioFeatures;
use crate::track_id::TrackId;

/// https://developer.spotify.com/documentation/web-api/reference/get-audio-features
pub async fn get_track_audio_features(
    track_id: TrackId,
    bearer: BearerToken,
) -> eyre::Result<TrackAudioFeatures> {
    let url = format!("https://api.spotify.com/v1/audio-features/{}", track_id);
    fetch(&url, bearer).await
}
