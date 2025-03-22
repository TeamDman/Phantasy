use crate::bearer_token::BearerToken;
use base64::Engine;
use eyre::OptionExt;
use eyre::Result;
use eyre::eyre;
use open::that as open_browser;
use rand::Rng;
use rand::distr::Alphanumeric;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tracing::debug;
use tracing::info;
use url::Url;

/// Load .env variables early
fn init_env() {
    dotenvy::dotenv().ok();
}

/// Read the required environment variable or error
fn var(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| eyre!("Missing env var: {}", name))
}

const BEARER_TOKEN_FILE: &'static str = "bearer_token.json";
pub async fn get_saved_token() -> Result<Option<BearerToken>> {
    if let Ok(token) = tokio::fs::read(BEARER_TOKEN_FILE).await {
        let token = serde_json::from_slice(&token)?;
        Ok(Some(token))
    } else {
        Ok(None)
    }
}

pub async fn save_token(token: &BearerToken) -> Result<()> {
    tokio::fs::write(BEARER_TOKEN_FILE, serde_json::to_string_pretty(token)?).await?;
    Ok(())
}

pub async fn get_bearer_token_via_pkce() -> Result<BearerToken> {
    debug!("Getting bearer token");
    if let Some(x) = get_saved_token().await? {
        return Ok(x);
    }
    init_env();

    let client_id = var("SPOTIFY_CLIENT_ID")?;
    let redirect_uri = var("SPOTIFY_REDIRECT_URI")?;
    let verifier = generate_code_verifier();
    let challenge = code_challenge(&verifier);

    let auth_url = Url::parse_with_params(
        "https://accounts.spotify.com/authorize",
        &[
            ("client_id", &client_id),
            ("response_type", &"code".to_string()),
            ("redirect_uri", &redirect_uri),
            ("code_challenge_method", &"S256".to_string()),
            ("code_challenge", &challenge),
            ("scope", &"user-library-read".to_string()), // adjust as needed
        ],
    )?;

    info!("Opening browser for auth");
    open_browser(auth_url.as_str())?;

    let code = listen_for_code().await?;

    let client = reqwest::Client::new();
    let resp = client
        .post("https://accounts.spotify.com/api/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect_uri),
            ("client_id", &client_id),
            ("code_verifier", &verifier),
        ])
        .send()
        .await?
        .error_for_status()?
        .json::<TokenResponse>()
        .await?;

    debug!("Access Token: len={}", resp.access_token.len());
    debug!("Scope: {}", resp.scope);
    debug!("Expires in: {}s", resp.expires_in);

    let rtn = BearerToken(resp.access_token);
    save_token(&rtn).await?;

    Ok(rtn)
}

fn generate_code_verifier() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(128)
        .map(char::from)
        .collect()
}

fn code_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash)
}

async fn listen_for_code() -> Result<String> {
    let redirect_uri = var("SPOTIFY_REDIRECT_URI")?;
    debug!("Listening for code on {}", redirect_uri);
    let addr = redirect_uri
        .strip_prefix("http://")
        .or_else(|| redirect_uri.strip_prefix("https://"))
        .ok_or_eyre("Invalid redirect URI")?;
    let listener = TcpListener::bind(addr).await?;
    let (mut socket, _) = listener.accept().await?;

    let mut buffer = [0; 1024];
    socket.read(&mut buffer).await?;
    let request = String::from_utf8_lossy(&buffer[..]);

    let code = request
        .split_whitespace()
        .nth(1)
        .and_then(|url| Url::parse(&format!("http://localhost{}", url)).ok())
        .and_then(|url| {
            url.query_pairs()
                .find(|(k, _)| k == "code")
                .map(|(_, v)| v.to_string())
        })
        .ok_or_else(|| eyre!("Failed to extract code from request"))?;

    let body = r#"
        <!DOCTYPE html>
        <html lang="en">
          <head><meta charset="UTF-8"><title>Spotify Auth</title></head>
          <body style="font-family:sans-serif;text-align:center;padding-top:3em">
            <h1>Phantasy</h1>
            ✅ <strong>Spotify auth complete.</strong><br/>You may close this window.
          </body>
        </html>
        "#;

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );

    socket.write_all(response.as_bytes()).await?;
    socket.write_all(response.as_bytes()).await?;

    Ok(code)
}

#[derive(Debug, Deserialize, Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    scope: String,
    expires_in: u64,
    refresh_token: Option<String>,
}
