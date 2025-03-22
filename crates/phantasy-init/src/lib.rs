pub fn init() -> eyre::Result<()> {
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

    dotenvy::dotenv()?;


    Ok(())
}
