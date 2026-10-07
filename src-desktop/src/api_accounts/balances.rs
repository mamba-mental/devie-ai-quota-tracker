use super::helpers::{detail, json, params, reading};
use crate::{
    credentials::Credentials,
    model::{AccountDetail, QuotaReading},
};
use chrono::{Datelike, NaiveDate, Utc};
use reqwest::Client;
use serde_json::{json as obj, Value};
use std::collections::{BTreeMap, BTreeSet};

const SHAPE: &str = "The provider returned incomplete or invalid account metrics.";

fn number(v: &Value) -> Option<f64> {
    let n = v.as_f64().or_else(|| v.as_str()?.parse().ok())?;
    n.is_finite().then_some(n)
}
fn metric(
    out: &mut Vec<AccountDetail>,
    v: &Value,
    field: &str,
    label: &str,
    kind: &str,
    unit: Option<&str>,
    scale: f64,
) {
    if let Some(n) = number(&v[field]) {
        let n = n / scale;
        if n.is_finite() {
            out.push(detail(field, label, &n.to_string(), kind, unit));
        }
    }
}
fn timestamp(out: &mut Vec<AccountDetail>, v: &Value, field: &str, label: &str, kind: &str) {
    if let Some(s) = v[field]
        .as_str()
        .filter(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok())
    {
        out.push(detail(field, label, s, kind, None));
    }
}
fn finish(out: Vec<AccountDetail>) -> Result<QuotaReading, String> {
    if out.is_empty() {
        Err(SHAPE.into())
    } else {
        Ok(reading(out))
    }
}
fn required<'a>(p: &'a BTreeMap<String, String>, field: &str) -> Result<&'a str, String> {
    p.get(field)
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("Configure the {field} account field first."))
}
fn month(p: &BTreeMap<String, String>) -> Result<String, String> {
    let value = p
        .get("month")
        .cloned()
        .unwrap_or_else(|| Utc::now().format("%Y-%m").to_string());
    if value.len() != 7 || NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d").is_err() {
        return Err("Month must use YYYY-MM.".into());
    }
    Ok(value)
}
fn period(p: &BTreeMap<String, String>) -> Result<(String, String), String> {
    let m = month(p)?;
    let start = NaiveDate::parse_from_str(&format!("{m}-01"), "%Y-%m-%d").map_err(|_| SHAPE)?;
    let (y, m) = if start.month() == 12 {
        (start.year() + 1, 1)
    } else {
        (start.year(), start.month() + 1)
    };
    let end = NaiveDate::from_ymd_opt(y, m, 1).ok_or(SHAPE)?;
    Ok((format!("{start}T00:00:00Z"), format!("{end}T00:00:00Z")))
}

pub async fn read(
    provider: &str,
    client: &Client,
    credentials: &Credentials,
) -> Result<QuotaReading, String> {
    let p = params(credentials)?;
    let key = &credentials.access_token;
    let get = |url: &str| client.get(url).bearer_auth(key);
    match provider {
        "openrouter" => {
            let mut out = Vec::new();
            let cap = json(get("https://openrouter.ai/api/v1/key")).await;
            if let Ok(v) = &cap {
                out.extend(openrouter_key(v));
            }
            let management = credentials
                .refresh_token
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(key);
            let credits = json(
                client
                    .get("https://openrouter.ai/api/v1/credits")
                    .bearer_auth(management),
            )
            .await;
            if let Ok(v) = &credits {
                let data = &v["data"];
                metric(
                    &mut out,
                    data,
                    "total_credits",
                    "Account credits purchased",
                    "money",
                    Some("USD"),
                    1.0,
                );
                metric(
                    &mut out,
                    data,
                    "total_usage",
                    "Account credits spent",
                    "usage",
                    Some("USD"),
                    1.0,
                );
                if let (Some(a), Some(b)) =
                    (number(&data["total_credits"]), number(&data["total_usage"]))
                {
                    if (a - b).is_finite() {
                        out.push(detail(
                            "account_remaining",
                            "Account credits remaining (derived)",
                            &(a - b).to_string(),
                            "balance",
                            Some("USD"),
                        ));
                    }
                }
            }
            if out.is_empty() {
                return Err("OpenRouter account metrics unavailable. Check the API key and management-key permission.".into());
            }
            if credits.is_err() {
                out.push(detail(
                    "account_balance_status",
                    "Account balance",
                    "Unavailable; management key required",
                    "status",
                    None,
                ));
            }
            if cap.is_err() {
                out.push(detail(
                    "key_status",
                    "API key limits",
                    "Unavailable for this key",
                    "status",
                    None,
                ));
            }
            finish(out)
        }
        "deepseek" => deepseek(&json(get("https://api.deepseek.com/user/balance")).await?),
        "vercel-ai-gateway" => {
            let mut req = get("https://ai-gateway.vercel.sh/v1/credits");
            if let Some(team) = p.get("team_id") {
                req = req.query(&[("teamId", team)]);
            }
            vercel(&json(req).await?)
        }
        "requesty" => {
            let v = json(get("https://api-v2.requesty.ai/v1/manage/org")).await?;
            let mut out = Vec::new();
            metric(
                &mut out,
                &v,
                "balance",
                "Organization balance",
                "balance",
                Some("USD"),
                1.0,
            );
            if out.is_empty() {
                return Err(SHAPE.into());
            }
            let (start, end) = period(&p)?;
            match json(
                get("https://api-v2.requesty.ai/v1/manage/org/usage").query(&[
                    ("start", start),
                    ("end", end),
                    ("resolution", "day".into()),
                ]),
            )
            .await
            {
                Ok(usage) => {
                    let sum = usage["usage"].as_object().and_then(|entries| {
                        entries.values().try_fold(0.0, |sum, v| {
                            number(&v["spend"])
                                .and_then(|n| (sum + n).is_finite().then_some(sum + n))
                        })
                    });
                    if let Some(sum) = sum {
                        out.push(detail(
                            "period_spend",
                            "Reported month spend",
                            &sum.to_string(),
                            "usage",
                            Some("USD"),
                        ));
                    } else {
                        out.push(detail(
                            "spend_status",
                            "Month spend",
                            "Unavailable: incomplete usage data",
                            "status",
                            None,
                        ));
                    }
                }
                Err(_) => out.push(detail(
                    "spend_status",
                    "Month spend",
                    "Unavailable; organization usage permission may be required",
                    "status",
                    None,
                )),
            }
            out.push(detail(
                "billing_month",
                "Billing month",
                &month(&p)?,
                "text",
                None,
            ));
            finish(out)
        }
        "siliconflow" => {
            let (url, unit) = match p.get("region").map(String::as_str) {
                Some("cn") => ("https://api.siliconflow.cn/v1/user/info", "CNY"),
                Some("global") => ("https://api.siliconflow.com/v1/user/info", "USD"),
                _ => return Err("Select SiliconFlow region: cn or global.".into()),
            };
            let v = json(get(url)).await?;
            if v["status"] == false {
                return Err("SiliconFlow declined the account query.".into());
            }
            let mut out = Vec::new();
            for (f, l) in [
                ("balance", "Granted balance"),
                ("chargeBalance", "Top-up balance"),
                ("totalBalance", "Total balance"),
            ] {
                metric(&mut out, &v["data"], f, l, "balance", Some(unit), 1.0);
            }
            finish(out)
        }
        "novita" => novita(
            &json(
                get("https://api.novita.ai/openapi/v1/billing/balance/detail")
                    .header("Content-Type", "application/json"),
            )
            .await?,
        ),
        "venice" => {
            let mut out = Vec::new();
            if let Ok(v) = json(get("https://api.venice.ai/api/v1/api_keys/rate_limits")).await {
                let d = &v["data"];
                for (f, l, u) in [
                    ("USD", "USD balance", "USD"),
                    ("DIEM", "DIEM remaining", "DIEM"),
                    ("BUNDLED_CREDITS", "Bundled credits", "USD"),
                ] {
                    metric(&mut out, &d["balances"], f, l, "balance", Some(u), 1.0);
                }
                timestamp(&mut out, d, "nextEpochBegins", "Next DIEM epoch", "reset");
                timestamp(&mut out, d, "keyExpiration", "API key expiry", "expiry");
                if let Some(b) = d["accessPermitted"].as_bool() {
                    out.push(detail(
                        "accessPermitted",
                        "API access permitted",
                        &b.to_string(),
                        "status",
                        None,
                    ));
                }
            }
            let admin = credentials
                .refresh_token
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(key);
            match json(
                client
                    .get("https://api.venice.ai/api/v1/billing/balance")
                    .bearer_auth(admin),
            )
            .await
            {
                Ok(v) => {
                    for (f, l, u) in [
                        ("diem", "Account DIEM remaining", "DIEM"),
                        ("usd", "Account USD balance", "USD"),
                        ("bundledCredits", "Account bundled credits", "USD"),
                        ("earnedCredits", "Earned credits", "USD"),
                    ] {
                        metric(&mut out, &v["balances"], f, l, "balance", Some(u), 1.0);
                    }
                    metric(
                        &mut out,
                        &v,
                        "diemEpochAllocation",
                        "DIEM epoch allocation",
                        "usage",
                        Some("DIEM"),
                        1.0,
                    );
                }
                Err(_) if !out.is_empty() => out.push(detail(
                    "billing_status",
                    "Full billing balance",
                    "Unavailable; admin key required",
                    "status",
                    None,
                )),
                Err(_) => {
                    return Err(
                        "Venice balance unavailable. Verify key access and admin permission."
                            .into(),
                    )
                }
            }
            finish(out)
        }
        "huggingface-zerogpu" => {
            hf(&json(get("https://huggingface.co/api/spaces/zero-gpu/quota")).await?)
        }
        "together" => together(client, credentials, &p).await,
        "fireworks" => {
            let account = required(&p, "account_id")?;
            if !account
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return Err("Invalid Fireworks account ID.".into());
            }
            let (start, end) = period(&p)?;
            let v = json(
                get(&format!(
                    "https://api.fireworks.ai/v1/accounts/{account}/billing/summary"
                ))
                .query(&[("startTime", start), ("endTime", end)]),
            )
            .await?;
            let mut out = fireworks(&v)?;
            out.push(detail(
                "billing_month",
                "Billing month",
                &month(&p)?,
                "text",
                None,
            ));
            finish(out)
        }
        "runpod" => {
            // Vendor runpodctl internal/api/graphql.go: fixed POST + Bearer, no key in URL.
            // Official GraphQL User schema and runpodctl user document these three fields.
            let v=json(client.post("https://api.runpod.io/graphql").bearer_auth(key).json(&obj!({"query":"query { myself { clientBalance currentSpendPerHr spendLimit } }","variables":{}}))).await?;
            if v.get("errors")
                .is_some_and(|e| !e.is_null() && e.as_array().map_or(true, |a| !a.is_empty()))
            {
                return Err("Runpod rejected the account query.".into());
            }
            let mut out = Vec::new();
            for (f, l, k, u) in [
                ("clientBalance", "Credit balance", "balance", "USD"),
                (
                    "currentSpendPerHr",
                    "Current hourly spend",
                    "usage",
                    "USD/hour",
                ),
                ("spendLimit", "Hourly spending limit", "money", "USD/hour"),
            ] {
                metric(&mut out, &v["data"]["myself"], f, l, k, Some(u), 1.0);
            }
            finish(out)
        }
        _ => Err("Unsupported balance provider.".into()),
    }
}

fn openrouter_key(v: &Value) -> Vec<AccountDetail> {
    let v = &v["data"];
    let mut out = Vec::new();
    for (f, l) in [
        ("limit", "API key budget"),
        ("limit_remaining", "Key budget remaining"),
        ("usage", "Key lifetime spend"),
        ("usage_daily", "Key UTC day spend"),
        ("usage_weekly", "Key UTC week spend"),
        ("usage_monthly", "Key UTC month spend"),
    ] {
        metric(
            &mut out,
            v,
            f,
            l,
            if f == "limit_remaining" {
                "balance"
            } else {
                "usage"
            },
            Some("USD"),
            1.0,
        );
    }
    if v.get("limit") == Some(&Value::Null) {
        out.push(detail(
            "limit",
            "API key budget",
            "Unlimited key cap; account balance still applies",
            "status",
            None,
        ));
    }
    if let Some(s) = v["limit_reset"]
        .as_str()
        .filter(|s| matches!(*s, "daily" | "weekly" | "monthly"))
    {
        out.push(detail(
            "limit_reset",
            "Key budget reset policy",
            s,
            "text",
            None,
        ));
    }
    for (f, l) in [
        ("used", "Free requests used today"),
        ("limit", "Free requests allowed today"),
        ("remaining", "Free requests remaining today"),
    ] {
        if let Some(n) = number(&v["free_model_daily_requests"][f]).filter(|n| *n >= 0.0) {
            out.push(detail(
                &format!("free_requests_{f}"),
                l,
                &n.to_string(),
                "usage",
                Some("requests"),
            ));
        }
    }
    out
}
fn deepseek(v: &Value) -> Result<QuotaReading, String> {
    let mut out = Vec::new();
    for balance in v["balance_infos"].as_array().ok_or(SHAPE)? {
        let unit = balance["currency"]
            .as_str()
            .filter(|c| matches!(*c, "CNY" | "USD"))
            .ok_or(SHAPE)?;
        for (f, l) in [
            ("total_balance", "Total balance"),
            ("granted_balance", "Unexpired granted balance"),
            ("topped_up_balance", "Top-up balance"),
        ] {
            if let Some(n) = number(&balance[f]) {
                out.push(detail(
                    &format!("{unit}_{f}"),
                    l,
                    &n.to_string(),
                    "balance",
                    Some(unit),
                ));
            }
        }
    }
    finish(out)
}
fn vercel(v: &Value) -> Result<QuotaReading, String> {
    let mut out = Vec::new();
    metric(
        &mut out,
        v,
        "balance",
        "Team Gateway balance",
        "balance",
        Some("USD"),
        1.0,
    );
    metric(
        &mut out,
        v,
        "total_used",
        "Team lifetime Gateway spend",
        "usage",
        Some("USD"),
        1.0,
    );
    finish(out)
}
fn novita(v: &Value) -> Result<QuotaReading, String> {
    let mut out = Vec::new();
    for (f, l, k) in [
        ("availableBalance", "Available balance", "balance"),
        ("cashBalance", "Top-up balance", "balance"),
        ("creditLimit", "Permitted debt limit", "money"),
        ("pendingCharges", "Pending charges", "money"),
        ("outstandingInvoices", "Outstanding invoices", "money"),
    ] {
        metric(&mut out, v, f, l, k, Some("USD"), 10000.0);
    }
    finish(out)
}
fn hf(v: &Value) -> Result<QuotaReading, String> {
    let base = number(&v["base"]).filter(|n| *n > 0.0).ok_or(SHAPE)?;
    let left = number(&v["current"])
        .filter(|n| *n >= 0.0 && *n <= base)
        .ok_or(SHAPE)?;
    let mut out = Vec::new();
    out.push(detail(
        "base",
        "ZeroGPU allowance",
        &base.to_string(),
        "usage",
        Some("GPU-seconds"),
    ));
    out.push(detail(
        "remaining",
        "ZeroGPU remaining",
        &left.to_string(),
        "usage",
        Some("GPU-seconds"),
    ));
    metric(
        &mut out,
        v,
        "overquotaUsed",
        "Paid ZeroGPU usage",
        "usage",
        Some("GPU-seconds"),
        1.0,
    );
    timestamp(&mut out, v, "resetsAt", "ZeroGPU reset", "reset");
    finish(out)
}

fn together_page(v: &Value, expected_month: &str) -> Result<(f64, Option<String>), String> {
    if v["currency"] != "USD" || v["billing_period"] != expected_month {
        return Err(SHAPE.into());
    }
    let mut sum = 0.0;
    for window in v["data"].as_array().ok_or(SHAPE)? {
        for item in window["line_items"].as_array().ok_or(SHAPE)? {
            sum += number(&item["cost"]).ok_or(SHAPE)?;
            if !sum.is_finite() {
                return Err(SHAPE.into());
            }
        }
    }
    let next = match v.get("next_cursor") {
        Some(Value::Null) => None,
        Some(Value::String(s)) if !s.is_empty() && s.len() <= 4096 => Some(s.clone()),
        _ => return Err(SHAPE.into()),
    };
    Ok((sum, next))
}
async fn together(
    client: &Client,
    c: &Credentials,
    p: &BTreeMap<String, String>,
) -> Result<QuotaReading, String> {
    let month = month(p)?;
    let org = required(p, "organization_id")?;
    let mut seen = BTreeSet::new();
    let mut cursor = None;
    let mut total = 0.0;
    for _ in 0..100 {
        let mut request = client
            .get("https://api.together.ai/v1/billing/usage")
            .bearer_auth(&c.access_token)
            .query(&[
                ("month", month.as_str()),
                ("organization_id", org),
                ("granularity", "day"),
                ("limit", "1000"),
            ]);
        if let Some(ref after) = cursor {
            request = request.query(&[("after", after)]);
        }
        let v = json(request).await?;
        if v["organization_id"] != org {
            return Err("Together returned another organization's report.".into());
        }
        let (subtotal, next) = together_page(&v, &month)?;
        total += subtotal;
        if !total.is_finite() {
            return Err(SHAPE.into());
        }
        match next {
            None => {
                return finish(vec![
                    detail(
                        "period_spend",
                        "Reported month spend",
                        &total.to_string(),
                        "usage",
                        Some("USD"),
                    ),
                    detail("billing_month", "Billing month", &month, "text", None),
                    detail(
                        "data_delay",
                        "Report freshness",
                        "Current month may lag 1 hour; previous months up to 24 hours",
                        "status",
                        None,
                    ),
                ])
            }
            Some(next) if seen.insert(next.clone()) => cursor = Some(next),
            Some(_) => {
                return Err(
                    "Together pagination repeated; no partial spend total was saved.".into(),
                )
            }
        }
    }
    Err("Together pagination exceeded the safe page limit; no partial total was saved.".into())
}
fn money(v: &Value) -> Result<(String, f64), String> {
    let currency = v["currencyCode"]
        .as_str()
        .filter(|s| s.len() == 3 && s.bytes().all(|b| b.is_ascii_uppercase()))
        .ok_or(SHAPE)?;
    // Protobuf JSON may omit zero-valued units or nanos, but must have a currency.
    let units = match v.get("units") {
        None => 0.0,
        Some(n) => number(n).filter(|n| n.fract() == 0.0).ok_or(SHAPE)?,
    };
    let nanos = match v.get("nanos") {
        None => 0.0,
        Some(n) => number(n)
            .filter(|n| n.fract() == 0.0 && n.abs() < 1e9)
            .ok_or(SHAPE)?,
    };
    if (units > 0.0 && nanos < 0.0) || (units < 0.0 && nanos > 0.0) {
        return Err(SHAPE.into());
    }
    Ok((currency.into(), units + nanos / 1e9))
}
fn fireworks(v: &Value) -> Result<Vec<AccountDetail>, String> {
    let items = v["lineItems"].as_array().ok_or(SHAPE)?;
    let mut totals = BTreeMap::<String, f64>::new();
    for item in items {
        let (currency, amount) = money(&item["totalCost"])?;
        let sum = totals.entry(currency).or_default();
        *sum += amount;
        if !sum.is_finite() {
            return Err(SHAPE.into());
        }
    }
    if items.is_empty() {
        return Ok(vec![detail(
            "period_spend_status",
            "Reported month spend",
            "No billing line items returned",
            "status",
            None,
        )]);
    }
    Ok(totals
        .into_iter()
        .map(|(currency, total)| {
            detail(
                &format!("period_spend_{currency}"),
                "Reported month spend",
                &total.to_string(),
                "usage",
                Some(&currency),
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn details(r: QuotaReading) -> Vec<AccountDetail> {
        r.identity.unwrap().details
    }
    #[test]
    fn novita_denominator_and_missing_values() {
        let d = details(
            novita(&obj!({"availableBalance":"10000","cashBalance":null,"creditLimit":"2500"}))
                .unwrap(),
        );
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].value, "1");
        assert_eq!(d[1].value, "0.25");
        assert!(novita(&obj!({"availableBalance":null})).is_err());
    }
    #[test]
    fn vercel_numeric_and_string_contracts() {
        assert_eq!(
            details(vercel(&obj!({"balance":"2.5","total_used":3})).unwrap())[0].value,
            "2.5"
        );
        assert!(vercel(&obj!({"balance":"NaN"})).is_err());
    }
    #[test]
    fn deepseek_currencies_stay_separate() {
        let d=details(deepseek(&obj!({"balance_infos":[{"currency":"USD","total_balance":"2"},{"currency":"CNY","total_balance":"3"}]})).unwrap());
        assert_eq!(d.len(), 2);
        assert_ne!(d[0].key, d[1].key);
    }
    #[test]
    fn hf_seconds_reject_invalid_ranges() {
        assert!(hf(&obj!({"base":2400,"current":1810,"resetsAt":null})).is_ok());
        for v in [
            obj!({"base":0,"current":0}),
            obj!({"base":100,"current":101}),
            obj!({"base":100,"current":-1}),
            obj!({"base":100}),
        ] {
            assert!(hf(&v).is_err());
        }
    }
    #[test]
    fn together_requires_explicit_end_and_complete_costs() {
        let v = obj!({"currency":"USD","billing_period":"2026-10","data":[{"line_items":[{"cost":"1.25"},{"cost":"2"}]}],"next_cursor":null});
        assert_eq!(together_page(&v, "2026-10").unwrap(), (3.25, None));
        assert!(together_page(&v, "2026-09").is_err());
        let mut bad = v.clone();
        bad.as_object_mut().unwrap().remove("next_cursor");
        assert!(together_page(&bad, "2026-10").is_err());
        bad = v;
        bad["data"][0]["line_items"][0]["cost"] = Value::Null;
        assert!(together_page(&bad, "2026-10").is_err());
    }
    #[test]
    fn fireworks_nanos_and_double_counting() {
        let v = obj!({"lineItems":[{"totalCost":{"currencyCode":"USD","units":"1","nanos":250000000}}],"usageBuckets":[{"lineItems":[{"totalCost":{"currencyCode":"USD","units":"1","nanos":250000000}}]}]});
        assert_eq!(fireworks(&v).unwrap()[0].value, "1.25");
        assert!(money(&obj!({"currencyCode":"USD","nanos":1000000000})).is_err());
        assert!(money(&obj!({"currencyCode":"USD","units":"1","nanos":-1})).is_err());
        assert!(fireworks(&obj!({"lineItems":[{}]})).is_err());
    }
    #[test]
    fn month_validation_and_year_rollover() {
        let mut p = BTreeMap::from([("month".into(), "2026-12".into())]);
        assert_eq!(period(&p).unwrap().1, "2027-01-01T00:00:00Z");
        p.insert("month".into(), "2026-13".into());
        assert!(month(&p).is_err());
    }
}
