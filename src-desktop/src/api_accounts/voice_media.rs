use super::helpers::{detail, json, params, reading};
use crate::{
    credentials::Credentials,
    model::{AccountDetail, QuotaAmount, QuotaReading, QuotaWindow},
};
use serde_json::{json as payload, Value};

pub async fn read(
    provider: &str,
    client: &reqwest::Client,
    credentials: &Credentials,
) -> Result<QuotaReading, String> {
    let options = params(credentials)?;
    let key = &credentials.access_token;
    match provider {
        "elevenlabs" => parse_eleven(&json(client.get("https://api.elevenlabs.io/v1/user/subscription").header("xi-api-key", key)).await?),
        "fish-audio" => {
            let package = json(client.get("https://api.fish.audio/wallet/self/package").bearer_auth(key)).await?;
            let mut request = client.get("https://api.fish.audio/wallet/self/api-credit").bearer_auth(key);
            if let Some(team) = options.get("team_id").filter(|s| !s.trim().is_empty()) {
                request = request.query(&[("team_id", team)]);
            }
            let wallet = json(request).await?;
            parse_fish(&package, &wallet)
        }
        "fal" => parse_fal(&json(client.get("https://api.fal.ai/v1/account/billing").query(&[("expand", "credits")]).header("Authorization", format!("Key {key}"))).await?),
        "runware" => parse_runware(&json(client.post("https://api.runware.ai/v1").bearer_auth(key).json(&payload!([{
            "taskType": "accountManagement", "taskUUID": uuid::Uuid::new_v4().to_string(), "operation": "getDetails"
        }]))).await?),
        "imagerouter" => parse_imagerouter(&json(client.get("https://api.imagerouter.io/v1/credits").bearer_auth(key)).await?),
        "deepgram" => {
            let project = options.get("project_id").filter(|s| !s.trim().is_empty()).ok_or("Deepgram requires a project_id in account settings.")?;
            if project.len() > 128 || !project.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') {
                return Err("Deepgram project_id must be an account project identifier.".into());
            }
            // Construct an encoded path segment; never permit a project ID to alter the host or route.
            let mut url = reqwest::Url::parse("https://api.deepgram.com/v1/projects/").map_err(|_| "Invalid provider URL.")?;
            url.path_segments_mut().map_err(|_| "Invalid provider URL.")?.pop_if_empty().push(project).push("balances");
            parse_deepgram(&json(client.get(url).header("Authorization", format!("Token {key}"))).await?)
        }
        "civitai" | "inworld" | "higgsfield" => Err("This provider's official account tooling needs a verified read contract before automatic polling can be enabled.".into()),
        _ => Err("Unsupported voice or media account provider.".into()),
    }
}

fn number(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str()?.parse().ok())
        .filter(|n| n.is_finite())
}

fn numeric(
    out: &mut Vec<AccountDetail>,
    v: &Value,
    key: &str,
    label: &str,
    kind: &str,
    unit: Option<&str>,
) {
    if let Some(n) = number(v) {
        out.push(detail(key, label, n, kind, unit));
    }
}

fn text(out: &mut Vec<AccountDetail>, v: &Value, key: &str, label: &str, kind: &str) {
    if let Some(s) = v
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 160 && !s.chars().any(char::is_control))
    {
        out.push(detail(key, label, s, kind, None));
    }
}

fn date(out: &mut Vec<AccountDetail>, v: &Value, key: &str, label: &str, kind: &str) {
    if let Some(s) = v
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
    {
        out.push(detail(key, label, s.to_rfc3339(), kind, None));
    }
}

fn finish(details: Vec<AccountDetail>) -> Result<QuotaReading, String> {
    if details.is_empty() {
        return Err("The provider response contained no recognized account telemetry.".into());
    }
    Ok(reading(details))
}

fn parse_eleven(v: &Value) -> Result<QuotaReading, String> {
    let mut d = Vec::new();
    text(&mut d, &v["tier"], "tier", "Subscription tier", "plan");
    text(
        &mut d,
        &v["status"],
        "status",
        "Subscription status",
        "status",
    );
    text(
        &mut d,
        &v["billing_period"],
        "billing_period",
        "Billing period",
        "period",
    );
    text(
        &mut d,
        &v["character_refresh_period"],
        "refresh_period",
        "Allowance refresh period",
        "period",
    );
    numeric(
        &mut d,
        &v["character_count"],
        "usage",
        "Characters used",
        "usage",
        Some("characters"),
    );
    numeric(
        &mut d,
        &v["character_limit"],
        "allowance",
        "Character allowance",
        "allowance",
        Some("characters"),
    );
    let reset = v["next_character_count_reset_unix"]
        .as_i64()
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map(|t| t.to_rfc3339());
    if let Some(t) = &reset {
        d.push(detail("reset", "Allowance resets", t, "reset", None));
    }
    if let Some(currency) = v["current_overage"]["currency"].as_str() {
        numeric(
            &mut d,
            &v["current_overage"]["amount"],
            "overage",
            "Current overage",
            "spend",
            Some(currency),
        );
    }
    let mut result = finish(d)?;
    if let Some(identity) = &mut result.identity {
        identity.plan = v["tier"].as_str().map(str::to_owned);
    }
    if let (Some(used), Some(total)) =
        (number(&v["character_count"]), number(&v["character_limit"]))
    {
        let percent = used / total * 100.0;
        if used >= 0.0 && total > 0.0 && percent.is_finite() {
            result.windows.push(QuotaWindow {
                key: "subscription".into(),
                label: "Subscription allowance".into(),
                used_percent: percent.clamp(0.0, 100.0),
                resets_at: reset,
                amount: Some(QuotaAmount {
                    used: Some(used),
                    total,
                    unit: Some("characters".into()),
                    overage: (used > total).then_some(used - total),
                }),
                ..Default::default()
            });
        }
    }
    Ok(result)
}

fn parse_fish(package: &Value, wallet: &Value) -> Result<QuotaReading, String> {
    let mut d = Vec::new();
    text(
        &mut d,
        &package["type"],
        "package_type",
        "Package type",
        "plan",
    );
    text(
        &mut d,
        &package["subscription_status"],
        "status",
        "Subscription status",
        "status",
    );
    text(
        &mut d,
        &package["billing_period"],
        "period",
        "Billing period",
        "period",
    );
    for (field, label, kind) in [
        ("total", "Package allowance", "allowance"),
        ("balance", "Package balance", "balance"),
        ("extra_balance", "Extra package balance", "balance"),
    ] {
        // The schema does not define these as USD or API wallet credits.
        numeric(
            &mut d,
            &package[field],
            &format!("package_{field}"),
            label,
            kind,
            Some("package units"),
        );
    }
    date(
        &mut d,
        &package["current_period_end"],
        "period_end",
        "Subscription period ends",
        "period_end",
    );
    date(
        &mut d,
        &package["finished_at"],
        "package_end",
        "Package ends",
        "expiry",
    );
    if let Some(cancel) = package["cancel_at_period_end"].as_bool() {
        d.push(detail(
            "cancel_at_period_end",
            "Cancels at period end",
            cancel,
            "status",
            None,
        ));
    }
    numeric(
        &mut d,
        &wallet["credit"],
        "api_credit",
        "API wallet balance",
        "balance",
        Some("API credits"),
    );
    numeric(
        &mut d,
        &wallet["cumulative_top_up"],
        "api_topups",
        "API wallet total top-ups",
        "deposit",
        Some("API credits"),
    );
    // Package unit/reset semantics are unverified: do not manufacture a quota window.
    finish(d)
}

fn parse_fal(v: &Value) -> Result<QuotaReading, String> {
    let mut d = Vec::new();
    let currency = v["credits"]["currency"]
        .as_str()
        .ok_or("fal did not return a balance currency.")?;
    numeric(
        &mut d,
        &v["credits"]["current_balance"],
        "balance",
        "API credit balance",
        "balance",
        Some(currency),
    );
    finish(d)
}

fn parse_runware(v: &Value) -> Result<QuotaReading, String> {
    if v.get("errors")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
    {
        return Err(
            "Runware rejected the account read. Check key permissions and account access.".into(),
        );
    }
    let account = v["data"]
        .as_array()
        .and_then(|items| {
            items.iter().find(|item| {
                item["taskType"] == "accountManagement" && item["operation"] == "getDetails"
            })
        })
        .ok_or("Runware did not return account details.")?;
    let currency = account["balance"]["currency"]
        .as_str()
        .ok_or("Runware did not return a balance currency.")?;
    let mut d = Vec::new();
    numeric(
        &mut d,
        &account["balance"]["amount"],
        "balance",
        "Account balance",
        "balance",
        Some(currency),
    );
    numeric(
        &mut d,
        &account["balance"]["freeBalance"],
        "free_balance",
        "Free credit balance",
        "balance",
        Some(currency),
    );
    for (key, label) in [
        ("today", "Today"),
        ("last7Days", "Last 7 days"),
        ("last30Days", "Last 30 days"),
        ("total", "Lifetime"),
    ] {
        numeric(
            &mut d,
            &account["usage"][key]["credits"],
            &format!("{key}_spend"),
            &format!("{label} spend"),
            "spend",
            Some(currency),
        );
        numeric(
            &mut d,
            &account["usage"][key]["requests"],
            &format!("{key}_requests"),
            &format!("{label} requests"),
            "usage",
            Some("requests"),
        );
    }
    // Never retain team roster, identifiers, or the apiKeys array.
    finish(d)
}

fn parse_imagerouter(v: &Value) -> Result<QuotaReading, String> {
    let mut d = Vec::new();
    for (key, label, kind) in [
        ("remaining_credits", "Credit balance", "balance"),
        ("credit_usage", "Total credit spend", "spend"),
        ("total_deposits", "Total deposits", "deposit"),
    ] {
        numeric(&mut d, &v[key], key, label, kind, Some("USD"));
    }
    finish(d)
}

fn parse_deepgram(v: &Value) -> Result<QuotaReading, String> {
    let balances = v["balances"]
        .as_array()
        .ok_or("Deepgram did not return project balances.")?;
    let mut d = Vec::new();
    for (index, balance) in balances.iter().enumerate() {
        if let Some(unit) = balance["units"].as_str() {
            numeric(
                &mut d,
                &balance["amount"],
                &format!("balance_{index}"),
                &format!("Project balance {}", index + 1),
                "balance",
                Some(unit),
            );
        }
    }
    finish(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fields(r: &QuotaReading) -> &[AccountDetail] {
        &r.identity.as_ref().unwrap().details
    }
    #[test]
    fn missing_is_unknown_and_numeric_strings_are_finite() {
        assert!(parse_imagerouter(&payload!({"remaining_credits":null})).is_err());
        assert!(number(&payload!("NaN")).is_none());
        let r = parse_imagerouter(&payload!({"remaining_credits":"2.50","credit_usage":"7.5"}))
            .unwrap();
        assert_eq!(fields(&r).len(), 2);
        assert_eq!(fields(&r)[0].unit.as_deref(), Some("USD"));
        assert!(r.windows.is_empty());
    }
    #[test]
    fn eleven_only_known_positive_cap_creates_window() {
        let r = parse_eleven(&payload!({"tier":"starter","status":"active","character_count":"25","character_limit":100,"next_character_count_reset_unix":1738356858})).unwrap();
        assert_eq!(r.windows[0].used_percent, 25.0);
        assert_eq!(
            r.windows[0].amount.as_ref().unwrap().unit.as_deref(),
            Some("characters")
        );
        assert!(r.windows[0].resets_at.is_some());
        assert!(fields(&r)
            .iter()
            .any(|d| d.key == "status" && d.value == "active"));
        for limit in [Value::Null, payload!(0), payload!(-1)] {
            assert!(
                parse_eleven(&payload!({"character_count":25,"character_limit":limit}))
                    .unwrap()
                    .windows
                    .is_empty()
            );
        }
    }
    #[test]
    fn fish_keeps_wallet_separate_and_period_end_is_not_reset() {
        let r = parse_fish(&payload!({"type":"pro","balance":80,"total":100,"current_period_end":"2026-11-01T00:00:00Z","subscription_status":"active"}), &payload!({"credit":"4.25","cumulative_top_up":null})).unwrap();
        assert!(r.windows.is_empty());
        assert!(fields(&r)
            .iter()
            .any(|d| d.key == "period_end" && d.kind == "period_end"));
        assert!(fields(&r)
            .iter()
            .any(|d| d.key == "api_credit" && d.unit.as_deref() == Some("API credits")));
        assert!(!fields(&r).iter().any(|d| d.key == "api_topups"));
    }
    #[test]
    fn runware_projects_safe_fields_and_reports_envelope_errors() {
        let r = parse_runware(&payload!({"data":[{"taskType":"accountManagement","operation":"getDetails","balance":{"amount":"12","currency":"USD"},"team":[{"email":"private@example.test"}],"apiKeys":[{"apiKey":"secret-placeholder"}]}]})).unwrap();
        assert_eq!(fields(&r).len(), 1);
        assert_eq!(fields(&r)[0].value, "12");
        assert!(
            parse_runware(&payload!({"errors":[{"message":"secret-placeholder"}]}))
                .unwrap_err()
                .find("secret-placeholder")
                .is_none()
        );
    }
    #[test]
    fn balances_preserve_currency_and_do_not_invent_zero() {
        assert!(parse_fal(&payload!({"credits":{"current_balance":1}})).is_err());
        let r = parse_fal(&payload!({"credits":{"current_balance":0,"currency":"USD"}})).unwrap();
        assert_eq!(fields(&r)[0].value, "0");
        let r = parse_deepgram(&payload!({"balances":[{"amount":"1.5","units":"USD"},{"amount":2,"units":"EUR"},{"amount":null,"units":"USD"}]})).unwrap();
        assert_eq!(fields(&r).len(), 2);
        assert_eq!(fields(&r)[1].unit.as_deref(), Some("EUR"));
        assert!(r.windows.is_empty());
        assert!(parse_deepgram(&payload!({"balances":[]})).is_err());
    }
}
