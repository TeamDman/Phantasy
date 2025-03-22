use eyre::eyre;
use phantasy_init::init;
use tracing::info;
use std::path::PathBuf;

/// Read the required environment variable or error
fn var(name: &str) -> eyre::Result<String> {
    std::env::var(name).map_err(|_| eyre!("Missing env var: {}", name))
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    init()?;

    let music_dir = PathBuf::try_from(var("MUSIC_DIR")?)?;
    let sample_path = PathBuf::try_from(var("SAMPLE_PATH")?)?;
    let sample_begin = var("SAMPLE_BEGIN")?.parse::<f32>()?;
    let sample_end = var("SAMPLE_END")?.parse::<f32>()?;

    let files_to_search = {
        let mut files = tokio::fs::read_dir(&music_dir).await?;
        let mut rtn = Vec::new();
        while let Some(file) = files.next_entry().await? {
            if file.path().extension().map_or(false, |ext| ext == "ogg") {
                rtn.push(file.path());
            }
        }
        rtn
    };
    info!("Found {} files to search in {}", files_to_search.len(), music_dir.display());



    Ok(())
}
