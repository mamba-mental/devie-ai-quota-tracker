use crate::{
    credentials::Credentials,
    model::{AccountDetail, QuotaReading, RemoteIdentity},
};
use std::{collections::BTreeMap, time::Duration};

pub fn params(credentials: &Credentials) -> Result<BTreeMap<String, String>, String> {
    match credentials.account_id.as_deref() {
        None => Ok(BTreeMap::new()),
        Some(value) => serde_json::from_str(value)
            .map_err(|_| "Account settings are invalid. Reconnect this account.".to_string()),
    }
}

pub async fn json(request: reqwest::RequestBuilder) -> Result<serde_json::Value, String> {
    let mut response = request
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| "The account API could not be reached.".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "The account API returned HTTP {}. Check your credentials and account permissions.",
            response.status().as_u16()
        ));
    }
    const MAX_BYTES: usize = 2 * 1024 * 1024;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BYTES as u64)
    {
        return Err("The account API response exceeded the size limit.".to_string());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The account API response was interrupted.".to_string())?
    {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err("The account API response exceeded the size limit.".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "The account API returned invalid JSON.".to_string())
}

pub fn detail(
    key: &str,
    label: &str,
    value: impl ToString,
    kind: &str,
    unit: Option<&str>,
) -> AccountDetail {
    AccountDetail {
        key: key.into(),
        label: label.into(),
        value: value.to_string(),
        kind: kind.into(),
        unit: unit.map(str::to_string),
    }
}

pub fn reading(details: Vec<AccountDetail>) -> QuotaReading {
    QuotaReading {
        source: "Account API".into(),
        identity: Some(RemoteIdentity {
            details,
            ..Default::default()
        }),
        windows: Vec::new(),
        reset_credits: None,
    }
}
