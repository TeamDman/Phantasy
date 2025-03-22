use phantasy_init::init;
use phantasy_spotify_api::auth::pkce::get_bearer_token_via_pkce;
use phantasy_spotify_api::get_track::get_track;
use phantasy_spotify_api::track_id::TrackId;

/// Read the required environment variable or error
fn var(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| eyre!("Missing env var: {}", name))
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    init()?;

    let bearer = get_bearer_token_via_pkce().await?;
    let track_id = TrackId("1NSNsucHrizvMEfer2tQ5D".to_string());

    let x = get_track(track_id, bearer).await?;
    println!("{:#?}", x);

    Ok(())
}