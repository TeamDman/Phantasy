use crate::bearer_token::BearerToken;
use crate::fetch::fetch;
use crate::track::Track;
use crate::track_id::TrackId;

/// https://developer.spotify.com/documentation/web-api/reference/get-track
pub async fn get_track(track_id: TrackId, bearer: BearerToken) -> eyre::Result<Track> {
    let url = format!("https://api.spotify.com/v1/tracks/{}", track_id);
    fetch(&url, bearer).await
}
