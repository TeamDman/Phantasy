use phantasy_spotify_api::auth::pkce::get_bearer_token_via_pkce;
use phantasy_spotify_api::get_track::get_track;
use phantasy_spotify_api::track_id::TrackId;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    init()?;

    let bearer = get_bearer_token_via_pkce().await?;
    let track_id = TrackId("1NSNsucHrizvMEfer2tQ5D".to_string());

    let x = get_track(track_id, bearer).await?;
    println!("{:#?}", x);

    Ok(())
}

fn init() -> eyre::Result<()> {
    color_eyre::install()?;

    let env_filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing::level_filters::LevelFilter::INFO.into())
        .from_env_lossy();
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_file(true)
        .with_line_number(true)
        .without_time()
        .init();

    Ok(())
}
