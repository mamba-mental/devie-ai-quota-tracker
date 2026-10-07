mod balances;
mod business;
pub mod catalog;
mod frontier;
pub mod helpers;
mod other;
mod subscriptions;
mod voice_media;
use crate::{credentials::Credentials, model::QuotaReading};
use std::{collections::BTreeMap, time::Duration};
pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Could not initialize account API client.".into())
}
pub fn validate_fields(
    provider: &str,
    key: &str,
    fields: &BTreeMap<String, String>,
    secondary: Option<&str>,
) -> Result<&'static catalog::CatalogEntry, String> {
    let entry =
        catalog::find(provider).ok_or_else(|| "Unsupported account API provider.".to_string())?;
    if (entry.api_key_required && key.trim().is_empty())
        || key.len() > 16384
        || key.chars().any(char::is_control)
    {
        return Err("Enter a valid account API key.".into());
    }
    if entry.secondary_secret_required && secondary.is_none_or(|value| value.trim().is_empty()) {
        return Err("This account requires a secondary secret.".into());
    }
    if secondary.is_some_and(|value| value.len() > 16384 || value.chars().any(char::is_control)) {
        return Err("The secondary secret is invalid.".into());
    }
    for field in entry.fields {
        if field.required
            && fields
                .get(field.id)
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err(format!("{} is required.", field.label));
        }
    }
    for (key, value) in fields {
        if !entry.fields.iter().any(|field| field.id == key)
            || value.len() > 1024
            || value.chars().any(char::is_control)
        {
            return Err("Invalid account settings.".into());
        }
    }
    Ok(entry)
}
pub async fn read(
    provider: &str,
    client: &reqwest::Client,
    credentials: &Credentials,
) -> Result<QuotaReading, String> {
    let entry =
        catalog::find(provider).ok_or_else(|| "Unsupported account API provider.".to_string())?;
    let id = provider
        .strip_prefix("api:")
        .ok_or_else(|| "Invalid account API provider.".to_string())?;
    let result = match entry.category {
        "subscriptions" => subscriptions::read(id, client, credentials).await,
        "voice_media" => voice_media::read(id, client, credentials).await,
        "balances" => balances::read(id, client, credentials).await,
        "business" => business::read(id, client, credentials).await,
        "frontier" => frontier::read(id, client, credentials).await,
        "other" => other::read(id, client, credentials).await,
        _ => Err("Unsupported account API provider.".into()),
    };
    let reading = result.map_err(|message| {
        if contains_secret(&message, credentials) {
            "The account API request failed.".to_string()
        } else {
            message
        }
    })?;
    validate_reading(&reading)?;
    let encoded = serde_json::to_string(&reading)
        .map_err(|_| "Invalid account API information.".to_string())?;
    if contains_secret(&encoded, credentials) {
        return Err("The account API returned unsafe account information.".into());
    }
    Ok(reading)
}

fn contains_secret(text: &str, credentials: &Credentials) -> bool {
    std::iter::once(credentials.access_token.as_str())
        .chain(credentials.refresh_token.as_deref())
        .filter(|secret| !secret.is_empty())
        .any(|secret| {
            text.contains(secret)
                || serde_json::to_string(secret)
                    .ok()
                    .is_some_and(|escaped| text.contains(&escaped[1..escaped.len() - 1]))
        })
}

fn validate_reading(reading: &QuotaReading) -> Result<(), String> {
    let information = !reading.windows.is_empty()
        || reading.identity.as_ref().is_some_and(|identity| {
            identity
                .plan
                .as_deref()
                .is_some_and(|plan| !plan.trim().is_empty())
                || identity
                    .details
                    .iter()
                    .any(|detail| !detail.value.trim().is_empty())
        });
    if !information {
        return Err("The account API returned no usable account information.".into());
    }
    Ok(())
}

pub fn connection_for(
    database: &crate::db::Database,
    provider: crate::model::Provider,
    id: Option<&str>,
    name: &str,
) -> Result<crate::model::NewConnection, String> {
    if let Some(id) = id {
        let existing = database
            .connection_by_id(id)?
            .ok_or_else(|| "The connection no longer exists.".to_string())?;
        if existing.provider != provider {
            return Err("The connection belongs to a different provider.".into());
        }
        return Ok(crate::model::NewConnection {
            id: existing.id,
            provider,
            label: existing.label,
            source_locator: existing.source_locator,
            identity: existing.identity,
        });
    }
    let id = crate::oauth::random_token(24);
    Ok(crate::model::NewConnection {
        source_locator: format!("account-api/{}/{id}", provider.as_str()),
        id,
        provider,
        label: name.to_string(),
        identity: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_readings_and_reflected_secrets_are_rejected() {
        assert!(validate_reading(&helpers::reading(Vec::new())).is_err());
        let detail = helpers::detail("balance", "Balance", "0", "balance", Some("USD"));
        assert!(validate_reading(&helpers::reading(vec![detail])).is_ok());
        let mut credentials = Credentials::default();
        credentials.access_token = "test-key-only".into();
        assert!(contains_secret(
            "Response echoed test-key-only",
            &credentials
        ));
        assert!(!contains_secret("HTTP 401", &credentials));
        credentials.access_token.clear();
        assert!(!contains_secret("Account information", &credentials));
    }

    #[test]
    fn providers_and_fields_are_allowlisted() {
        for entry in catalog::CATALOG {
            let provider = crate::model::Provider::from_db(entry.id).unwrap();
            assert_eq!(
                serde_json::from_str::<crate::model::Provider>(
                    &serde_json::to_string(&provider).unwrap()
                )
                .unwrap(),
                provider
            );
            assert_eq!(provider.as_str(), entry.id);
        }
        assert!(crate::model::Provider::from_db("api:unknown").is_none());
        assert!(validate_fields("api:elevenlabs", "", &BTreeMap::new(), None).is_err());
        assert!(validate_fields(
            "api:elevenlabs",
            "valid",
            &BTreeMap::from([("url".into(), "https://untrusted.invalid".into())]),
            None
        )
        .is_err());
    }
}
