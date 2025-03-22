use crate::bearer_token::BearerToken;

pub async fn fetch<T>(url: &str, bearer: BearerToken) -> eyre::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let client = reqwest::Client::new();
    let res = client
        .get(url)
        .bearer_auth(bearer.0)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    match serde_json::from_str(&res) {
        Ok(x) => Ok(x),
        Err(e) => Err(eyre::Error::new(e).wrap_err(format!("Failed to deserialize:\n{}", res))),
    }
}
