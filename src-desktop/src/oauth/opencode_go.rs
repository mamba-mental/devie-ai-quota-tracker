use chrono::DateTime;
use reqwest::{Client, StatusCode};
use serde::Deserialize;

use crate::{
    credentials::Credentials,
    db::Database,
    model::{NewConnection, Provider, QuotaReading, QuotaWindow},
};

const USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";
const INVALID_USAGE: &str = "OpenCode Go returned invalid usage data. Try refreshing later.";

#[derive(Deserialize)]
struct UsageResponse {
    usage: UsageWindows,
}

#[derive(Deserialize)]
struct UsageWindows {
    rolling: UsageWindow,
    weekly: UsageWindow,
    monthly: UsageWindow,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum UsageStatus {
    Ok,
    RateLimited,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageWindow {
    #[serde(rename = "status")]
    _status: UsageStatus,
    percent: f64,
    resets_at: String,
}

pub fn connection_for(
    database: &Database,
    existing_id: Option<&str>,
) -> Result<NewConnection, String> {
    if let Some(id) = existing_id {
        let existing = database
            .connection_by_id(id)?
            .filter(|connection| connection.provider == Provider::OpenCodeGo)
            .ok_or_else(|| "This OpenCode Go connection no longer exists.".to_string())?;
        return Ok(NewConnection {
            id: existing.id,
            provider: existing.provider,
            label: existing.label,
            source_locator: existing.source_locator,
            identity: existing.identity,
        });
    }
    let id = uuid::Uuid::new_v4().to_string();
    Ok(NewConnection {
        source_locator: format!("api-key/opencode-go/{id}"),
        id,
        provider: Provider::OpenCodeGo,
        label: "OpenCode Go".to_string(),
        identity: None,
    })
}

pub async fn usage(client: &Client, credentials: &Credentials) -> Result<QuotaReading, String> {
    let key = &credentials.access_token;
    if key.is_empty() || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err("Enter a valid OpenCode Go API key to sign in.".to_string());
    }
    let response = client
        .get(USAGE_URL)
        .bearer_auth(key)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| "OpenCode Go could not be reached. Try again.".to_string())?;
    check_status(response.status())?;
    let bytes = response
        .bytes()
        .await
        .map_err(|_| INVALID_USAGE.to_string())?;
    parse_usage(&bytes)
}

fn check_status(status: StatusCode) -> Result<(), String> {
    match status {
        StatusCode::OK => Ok(()),
        StatusCode::UNAUTHORIZED => Err("OpenCode Go rejected this API key. Sign in again with a valid key.".to_string()),
        StatusCode::FORBIDDEN => Err("This key has no OpenCode Go subscription. Sign in with a key from your subscribed workspace.".to_string()),
        StatusCode::TOO_MANY_REQUESTS => Err("OpenCode Go is limiting usage checks. Try again later.".to_string()),
        _ => Err(format!("OpenCode Go could not return usage (HTTP {}). Try again later.", status.as_u16())),
    }
}

fn parse_usage(bytes: &[u8]) -> Result<QuotaReading, String> {
    let response: UsageResponse =
        serde_json::from_slice(bytes).map_err(|_| INVALID_USAGE.to_string())?;
    let windows = [
        ("rolling", "Rolling", response.usage.rolling),
        ("weekly", "Weekly", response.usage.weekly),
        ("monthly", "Monthly", response.usage.monthly),
    ]
    .into_iter()
    .map(|(key, label, window)| {
        if !window.percent.is_finite()
            || window.percent < 0.0
            || DateTime::parse_from_rfc3339(&window.resets_at).is_err()
        {
            return Err(INVALID_USAGE.to_string());
        }
        Ok(QuotaWindow {
            key: key.to_string(),
            label: label.to_string(),
            used_percent: window.percent,
            resets_at: Some(window.resets_at),
            ..QuotaWindow::default()
        })
    })
    .collect::<Result<Vec<_>, String>>()?;
    Ok(QuotaReading {
        source: "OpenCode Go usage API".to_string(),
        identity: None,
        windows,
        reset_credits: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> serde_json::Value {
        json!({"usage": {
            "rolling": {"status":"ok","percent":12.5,"resetsAt":"2026-10-07T12:00:00Z"},
            "weekly": {"status":"rate-limited","percent":100,"resetsAt":"2026-10-10T12:00:00Z"},
            "monthly": {"status":"ok","percent":0,"resetsAt":"2026-11-01T12:00:00Z"}
        }})
    }

    #[test]
    fn parses_reported_windows_without_inventing_balances() {
        let reading = parse_usage(&serde_json::to_vec(&sample()).unwrap()).unwrap();
        assert_eq!(reading.windows.len(), 3);
        assert_eq!(reading.windows[0].used_percent, 12.5);
        assert_eq!(reading.windows[1].used_percent, 100.0);
        assert_eq!(reading.windows[2].used_percent, 0.0);
        assert_eq!(
            reading.windows[0].resets_at.as_deref(),
            Some("2026-10-07T12:00:00Z")
        );
        assert!(reading.identity.is_none());
        assert!(reading
            .windows
            .iter()
            .all(|window| window.amount.is_none() && !window.paid && !window.unlimited));
    }

    #[test]
    fn rejects_missing_and_malformed_usage_without_reflecting_input() {
        for value in [
            json!({}),
            json!({"usage":null}),
            json!({"usage":{"rolling":null}}),
        ] {
            assert_eq!(
                parse_usage(&serde_json::to_vec(&value).unwrap()).unwrap_err(),
                INVALID_USAGE
            );
        }
        for (field, value) in [
            ("status", json!("unknown")),
            ("percent", json!(-1)),
            ("percent", json!("secret-key")),
            ("resetsAt", json!("secret-key")),
        ] {
            let mut payload = sample();
            payload["usage"]["rolling"][field] = value;
            assert_eq!(
                parse_usage(&serde_json::to_vec(&payload).unwrap()).unwrap_err(),
                INVALID_USAGE
            );
        }
        assert_eq!(
            parse_usage(b"not-json-secret-key").unwrap_err(),
            INVALID_USAGE
        );
    }

    #[test]
    fn distinguishes_auth_entitlement_and_rate_limit_failures() {
        assert!(check_status(StatusCode::OK).is_ok());
        assert!(check_status(StatusCode::UNAUTHORIZED)
            .unwrap_err()
            .contains("Sign in"));
        assert!(check_status(StatusCode::FORBIDDEN)
            .unwrap_err()
            .contains("subscription"));
        assert!(check_status(StatusCode::TOO_MANY_REQUESTS)
            .unwrap_err()
            .contains("limiting"));
        assert!(check_status(StatusCode::INTERNAL_SERVER_ERROR).is_err());
    }

    #[test]
    fn reconnect_keeps_connection_identity_and_preferences() {
        let root = tempfile::tempdir().unwrap();
        let database = Database::open(root.path().join("test.sqlite")).unwrap();
        let initial = connection_for(&database, None).unwrap();
        database.upsert_connections(&[initial.clone()]).unwrap();
        database
            .set_custom_label(&initial.id, Some("Work"))
            .unwrap();
        database.set_enabled(&initial.id, false).unwrap();
        let renewed = connection_for(&database, Some(&initial.id)).unwrap();
        database.upsert_connections(&[renewed.clone()]).unwrap();
        assert_eq!(initial.id, renewed.id);
        assert_eq!(initial.source_locator, renewed.source_locator);
        let loaded = database.connection_by_id(&initial.id).unwrap().unwrap();
        assert_eq!(loaded.custom_label.as_deref(), Some("Work"));
        assert!(!loaded.enabled);
        assert_ne!(initial.id, connection_for(&database, None).unwrap().id);
    }

    #[test]
    fn refuses_replacing_another_provider_or_missing_connection() {
        let root = tempfile::tempdir().unwrap();
        let database = Database::open(root.path().join("test.sqlite")).unwrap();
        let mut other = connection_for(&database, None).unwrap();
        other.provider = Provider::Claude;
        database.upsert_connections(&[other.clone()]).unwrap();
        assert!(connection_for(&database, Some(&other.id)).is_err());
        assert!(connection_for(&database, Some("missing")).is_err());
    }
}
