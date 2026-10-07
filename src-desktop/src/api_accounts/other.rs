//! Read-only account contracts verified against official provider documentation.
//! See .local/research/other-routers.md. Never infer a subscription from a wallet.
use super::helpers::{detail, json, params, reading};
use crate::{
    credentials::Credentials,
    model::{AccountDetail, QuotaAmount, QuotaReading, QuotaWindow},
};
use serde_json::Value;

pub async fn read(
    provider: &str,
    client: &reqwest::Client,
    credentials: &Credentials,
) -> Result<QuotaReading, String> {
    let _options = params(credentials)?;
    let key = &credentials.access_token;
    // Every URL is literal: account settings cannot redirect credential delivery.
    match provider {
        "navyai" => {
            parse_navy(&json(client.get("https://api.navy/v1/usage").bearer_auth(key)).await?)
        }
        "ainative" => {
            let balance = json(
                client
                    .get("https://api.ainative.studio/api/v1/credits/balance")
                    .bearer_auth(key),
            )
            .await?;
            let subscription = json(
                client
                    .get("https://api.ainative.studio/api/v1/subscription/public")
                    .bearer_auth(key),
            )
            .await?;
            parse_ainative(&balance, &subscription)
        }
        "pollinations" => parse_pollinations(
            &json(
                client
                    .get("https://gen.pollinations.ai/account/balance")
                    .bearer_auth(key),
            )
            .await?,
        ),
        "ai-horde" => parse_horde(
            &json(
                client
                    .get("https://aihorde.net/api/v2/find_user")
                    .header("apikey", key)
                    .header("Client-Agent", "Devie:1:account-monitor"),
            )
            .await?,
        ),
        "bazaarlink" => {
            let balance = json(
                client
                    .get("https://api.bazaarlink.ai/v1/credits")
                    .bearer_auth(key),
            )
            .await?;
            let usage = json(
                client
                    .get("https://api.bazaarlink.ai/v1/usage")
                    .query(&[("period", "month")])
                    .bearer_auth(key),
            )
            .await?;
            parse_bazaarlink(&balance, &usage)
        }
        // Routeway requires a MANAGEMENT key, not an inference API key.
        "routeway" => parse_routeway(
            &json(
                client
                    .get("https://api.routeway.ai/v1/account/balance")
                    .bearer_auth(key),
            )
            .await?,
        ),
        _ => Err("This provider has no verified account API adapter.".into()),
    }
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|n| n.is_finite())
}

fn numeric(
    out: &mut Vec<AccountDetail>,
    value: &Value,
    key: &str,
    label: &str,
    kind: &str,
    unit: &str,
    signed: bool,
) {
    if let Some(n) = number(value).filter(|n| signed || *n >= 0.0) {
        out.push(detail(key, label, n, kind, Some(unit)));
    }
}

fn text(out: &mut Vec<AccountDetail>, value: &Value, key: &str, label: &str, kind: &str) {
    if let Some(s) = value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 160 && !s.chars().any(char::is_control))
    {
        out.push(detail(key, label, s, kind, None));
    }
}

fn date(value: &Value) -> Option<String> {
    value
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.to_rfc3339())
}

fn date_detail(out: &mut Vec<AccountDetail>, value: &Value, key: &str, label: &str, kind: &str) {
    if let Some(s) = date(value) {
        out.push(detail(key, label, s, kind, None));
    }
}

fn finish(details: Vec<AccountDetail>) -> Result<QuotaReading, String> {
    if details.is_empty() {
        return Err("The provider response contained no recognized account telemetry.".into());
    }
    Ok(reading(details))
}

fn window(
    out: &mut QuotaReading,
    key: &str,
    label: &str,
    used: &Value,
    total: &Value,
    reset: Option<String>,
    unit: &str,
) {
    if let (Some(used), Some(total)) = (number(used), number(total)) {
        if used >= 0.0 && total > 0.0 {
            let percent = used / total * 100.0;
            if !percent.is_finite() {
                return;
            }
            out.windows.push(QuotaWindow {
                key: key.into(),
                label: label.into(),
                used_percent: percent.clamp(0.0, 100.0),
                resets_at: reset,
                amount: Some(QuotaAmount {
                    used: Some(used),
                    total,
                    unit: Some(unit.into()),
                    overage: (used > total).then_some(used - total),
                }),
                ..Default::default()
            });
        }
    }
}

fn parse_navy(v: &Value) -> Result<QuotaReading, String> {
    let mut d = Vec::new();
    text(&mut d, &v["plan"], "plan", "Plan", "plan");
    numeric(
        &mut d,
        &v["limits"]["tokens_per_day"],
        "daily_limit",
        "Daily token allowance",
        "allowance",
        "tokens",
        false,
    );
    numeric(
        &mut d,
        &v["usage"]["tokens_used_today"],
        "daily_used",
        "Tokens used today",
        "usage",
        "tokens",
        false,
    );
    numeric(
        &mut d,
        &v["usage"]["tokens_remaining_today"],
        "daily_remaining",
        "Tokens remaining today",
        "balance",
        "tokens",
        false,
    );
    date_detail(
        &mut d,
        &v["usage"]["resets_at_utc"],
        "daily_reset",
        "Daily allowance resets",
        "reset",
    );
    for (field, label, kind) in [
        ("limit", "Requests per minute limit", "allowance"),
        ("used", "Requests in current minute window", "usage"),
        (
            "remaining",
            "Requests remaining in minute window",
            "balance",
        ),
    ] {
        numeric(
            &mut d,
            &v["rate_limits"]["per_minute"][field],
            &format!("rpm_{field}"),
            label,
            kind,
            "requests",
            false,
        );
    }
    numeric(
        &mut d,
        &v["rate_limits"]["per_minute"]["resets_in_ms"],
        "rpm_reset_in",
        "Minute window resets in",
        "duration",
        "milliseconds",
        false,
    );
    let mut r = finish(d)?;
    if let Some(identity) = r.identity.as_mut() {
        identity.plan = identity
            .details
            .iter()
            .find(|d| d.key == "plan")
            .map(|d| d.value.clone());
    }
    window(
        &mut r,
        "daily_tokens",
        "Daily tokens",
        &v["usage"]["tokens_used_today"],
        &v["limits"]["tokens_per_day"],
        date(&v["usage"]["resets_at_utc"]),
        "tokens",
    );
    Ok(r)
}

fn parse_ainative(balance: &Value, subscription: &Value) -> Result<QuotaReading, String> {
    if subscription.get("success") == Some(&Value::Bool(false)) {
        return Err("The provider could not return subscription details.".into());
    }
    let mut d = Vec::new();
    for (field, label, kind) in [
        (
            "total_credits",
            "Credits available this period",
            "allowance",
        ),
        ("used_credits", "Credits used this period", "usage"),
        ("remaining_credits", "Credits remaining", "balance"),
    ] {
        numeric(
            &mut d,
            &balance[field],
            field,
            label,
            kind,
            "credits",
            false,
        );
    }
    text(&mut d, &balance["plan"], "plan", "Plan tier", "plan");
    date_detail(
        &mut d,
        &balance["period_start"],
        "period_start",
        "Credit period starts",
        "period",
    );
    date_detail(
        &mut d,
        &balance["period_end"],
        "period_end",
        "Credit period ends",
        "period",
    );
    let s = &subscription["data"];
    text(
        &mut d,
        &s["plan_name"],
        "plan_name",
        "Subscription plan",
        "plan",
    );
    text(
        &mut d,
        &s["status"],
        "subscription_status",
        "Subscription status",
        "status",
    );
    text(
        &mut d,
        &s["billing_cycle"],
        "billing_cycle",
        "Billing cycle",
        "period",
    );
    date_detail(
        &mut d,
        &s["renewal_date"],
        "renewal",
        "Subscription renewal",
        "renewal",
    );
    let mut r = finish(d)?;
    if let Some(identity) = r.identity.as_mut() {
        identity.plan = identity
            .details
            .iter()
            .find(|d| d.key == "plan_name")
            .or_else(|| identity.details.iter().find(|d| d.key == "plan"))
            .map(|d| d.value.clone());
    }
    // period_end is a billing boundary, not proof that all purchased credits reset.
    window(
        &mut r,
        "period_credits",
        "Credits this period",
        &balance["used_credits"],
        &balance["total_credits"],
        None,
        "credits",
    );
    Ok(r)
}

fn parse_pollinations(v: &Value) -> Result<QuotaReading, String> {
    let mut d = Vec::new();
    numeric(
        &mut d,
        &v["balance"],
        "visible_balance",
        "Available balance for this key",
        "balance",
        "Pollen",
        true,
    );
    for (field, label) in [
        ("total", "Account Pollen balance"),
        ("tier", "Account earned Pollen"),
        ("paid", "Account paid Pollen"),
    ] {
        numeric(
            &mut d,
            &v["accountBalance"][field],
            &format!("account_{field}"),
            label,
            "balance",
            "Pollen",
            true,
        );
    }
    // balance is the key's remaining budget when budgeted; never an allowance denominator.
    finish(d)
}

fn parse_horde(v: &Value) -> Result<QuotaReading, String> {
    let mut d = Vec::new();
    numeric(
        &mut d,
        &v["kudos"],
        "kudos",
        "Kudos balance",
        "balance",
        "kudos",
        true,
    );
    numeric(
        &mut d,
        &v["monthly_kudos"]["amount"],
        "monthly_kudos",
        "Monthly recurring Kudos",
        "allowance",
        "kudos",
        false,
    );
    date_detail(
        &mut d,
        &v["monthly_kudos"]["last_received"],
        "last_kudos",
        "Last monthly Kudos received",
        "date",
    );
    numeric(
        &mut d,
        &v["concurrency"],
        "concurrency",
        "Concurrent generation limit",
        "allowance",
        "requests",
        false,
    );
    // Kudos are priority credits, not currency or a bounded subscription quota.
    finish(d)
}

fn parse_bazaarlink(balance: &Value, usage: &Value) -> Result<QuotaReading, String> {
    let mut d = Vec::new();
    numeric(
        &mut d,
        &balance["data"]["total_credits"],
        "balance",
        "Available balance",
        "balance",
        "USD",
        true,
    );
    numeric(
        &mut d,
        &balance["data"]["total_usage"],
        "lifetime_spend",
        "Lifetime spend",
        "spend",
        "USD",
        false,
    );
    numeric(
        &mut d,
        &usage["totals"]["spend"],
        "month_spend",
        "Month spend",
        "spend",
        "USD",
        false,
    );
    numeric(
        &mut d,
        &usage["totals"]["requests"],
        "month_requests",
        "Month requests",
        "usage",
        "requests",
        false,
    );
    numeric(
        &mut d,
        &usage["totals"]["tokens"],
        "month_tokens",
        "Month tokens",
        "usage",
        "tokens",
        false,
    );
    date_detail(
        &mut d,
        &usage["since"],
        "usage_since",
        "Usage range starts",
        "period",
    );
    date_detail(
        &mut d,
        &usage["until"],
        "usage_until",
        "Usage range ends",
        "period",
    );
    // total_credits is ALREADY remaining USD. Never subtract total_usage.
    finish(d)
}

fn parse_routeway(v: &Value) -> Result<QuotaReading, String> {
    let mut d = Vec::new();
    numeric(
        &mut d,
        &v["balance"],
        "balance",
        "Credit balance",
        "balance",
        "USD",
        true,
    );
    finish(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fields(r: &QuotaReading) -> &[AccountDetail] {
        &r.identity.as_ref().unwrap().details
    }
    #[test]
    fn navy_real_cap_and_reset_make_window() {
        let r = parse_navy(&json!({"plan":"free","limits":{"tokens_per_day":100},"usage":{"tokens_used_today":25,"tokens_remaining_today":75,"resets_at_utc":"2026-10-08T00:00:00Z"}})).unwrap();
        assert_eq!(r.windows[0].used_percent, 25.0);
        assert!(r.windows[0].resets_at.is_some());
        assert_eq!(r.identity.unwrap().plan.as_deref(), Some("free"));
    }
    #[test]
    fn navy_missing_zero_or_negative_cap_never_fakes_percentage() {
        for cap in [Value::Null, json!(0), json!(-5)] {
            let r = parse_navy(
                &json!({"limits":{"tokens_per_day":cap},"usage":{"tokens_used_today":25}}),
            )
            .unwrap();
            assert!(r.windows.is_empty());
        }
        let r = parse_navy(&json!({"plan":"free","limits":{"tokens_per_day":100},"usage":{"tokens_used_today":-2}})).unwrap();
        assert!(r.windows.is_empty());
        assert!(fields(&r).iter().all(|d| d.key != "daily_used"));
    }
    #[test]
    fn ainative_period_is_not_automatically_a_credit_reset() {
        let r = parse_ainative(&json!({"total_credits":100,"used_credits":30,"remaining_credits":70,"period_end":"2026-11-01T00:00:00Z"}), &json!({"success":true,"data":{"tier":"free","renewal_date":null}})).unwrap();
        assert_eq!(r.windows[0].used_percent, 30.0);
        assert!(r.windows[0].resets_at.is_none());
        assert!(fields(&r).iter().all(|d| d.key != "renewal"));
        let paid = parse_ainative(
            &json!({"remaining_credits":10}),
            &json!({"data":{"plan_name":"Pro","renewal_date":"2026-11-01T00:00:00Z"}}),
        )
        .unwrap();
        assert!(fields(&paid).iter().any(|d| d.kind == "renewal"));
        assert!(
            parse_ainative(&json!({"remaining_credits":10}), &json!({"success":false})).is_err()
        );
    }
    #[test]
    fn balances_preserve_units_and_negative_values_without_windows() {
        let r = parse_routeway(&json!({"balance":"-1.25"})).unwrap();
        assert_eq!(fields(&r)[0].value, "-1.25");
        assert_eq!(fields(&r)[0].unit.as_deref(), Some("USD"));
        assert!(r.windows.is_empty());
        assert!(parse_routeway(&json!({"balance":"NaN"})).is_err());
        assert!(parse_routeway(&json!({"balance":null})).is_err());
        assert!(parse_routeway(&json!({"error":"secret"})).is_err());
    }
    #[test]
    fn pollinations_key_budget_and_account_balance_are_separate() {
        let r = parse_pollinations(
            &json!({"balance":3,"accountBalance":{"total":30,"tier":10,"paid":20}}),
        )
        .unwrap();
        assert_eq!(fields(&r)[0].value, "3");
        assert_eq!(fields(&r)[1].value, "30");
        assert!(fields(&r)
            .iter()
            .all(|d| d.unit.as_deref() == Some("Pollen")));
        assert!(r.windows.is_empty());
        assert_eq!(
            fields(&parse_pollinations(&json!({"balance":3})).unwrap()).len(),
            1
        );
        assert!(parse_pollinations(&json!({"balance":null})).is_err());
    }
    #[test]
    fn horde_kudos_are_not_dollars_or_a_monthly_cap() {
        let r =
            parse_horde(&json!({"kudos":-5,"monthly_kudos":{"amount":100,"last_received":null}}))
                .unwrap();
        assert_eq!(fields(&r)[0].unit.as_deref(), Some("kudos"));
        assert_eq!(fields(&r)[0].value, "-5");
        assert!(r.windows.is_empty());
        assert!(parse_horde(&json!({})).is_err());
    }
    #[test]
    fn bazaarlink_total_credits_is_remaining_balance() {
        let r = parse_bazaarlink(
            &json!({"data":{"total_credits":42.5,"total_usage":7.83}}),
            &json!({"totals":{"spend":2,"tokens":100}}),
        )
        .unwrap();
        assert_eq!(fields(&r)[0].value, "42.5");
        assert_eq!(fields(&r)[0].unit.as_deref(), Some("USD"));
        assert!(r.windows.is_empty());
        assert!(parse_bazaarlink(&json!({"data":{"total_credits":null}}), &json!({})).is_err());
    }
}
