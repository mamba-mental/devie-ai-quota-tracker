//! Read-only account reports. No inference requests and no consumer-plan inference.
//! Contracts: see .local/research/frontier.md and provider links below.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{Datelike, SecondsFormat, Utc};
use reqwest::Client;
use serde_json::{json as value, Value};

use super::helpers::{detail, json, params, reading};
use crate::{
    credentials::Credentials,
    model::{AccountDetail, QuotaReading},
};

const SCHEMA: &str =
    "The account API returned an incomplete or unsupported report. No total was recorded.";
const MAX_PAGES: usize = 100;

pub async fn read(
    provider: &str,
    client: &Client,
    credentials: &Credentials,
) -> Result<QuotaReading, String> {
    let settings = params(credentials)?;
    if provider != "alibaba-billing" && credentials.access_token.trim().is_empty() {
        return Err("An account reporting credential is required; an inference key may not have billing access.".into());
    }
    if credentials.expires_at.is_some_and(|at| at <= Utc::now()) {
        return Err(
            "The account access token has expired. Reconnect with a current reporting credential."
                .into(),
        );
    }
    let details = match provider {
        "openai-api" => costs(client, credentials, false).await?,
        "anthropic-api" => costs(client, credentials, true).await?,
        "xai-api" => xai(client, credentials, &settings).await?,
        "mistral-api" => mistral(client, credentials).await?,
        "perplexity-enterprise" => perplexity(client, credentials).await?,
        "groq-enterprise" => groq(client, credentials).await?,
        "azure-billing" => azure(client, credentials, &settings).await?,
        "alibaba-billing" => alibaba(&settings).await?,
        _ => return Err("Unsupported account reporting provider.".into()),
    };
    Ok(reading(details))
}

fn number(v: &Value) -> Result<f64, String> {
    let n = v
        .as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .ok_or(SCHEMA)?;
    if n.is_finite() {
        Ok(n)
    } else {
        Err(SCHEMA.into())
    }
}

fn currency(v: &Value) -> Result<String, String> {
    let s = v.as_str().ok_or(SCHEMA)?;
    if s.len() != 3 || !s.bytes().all(|c| c.is_ascii_alphabetic()) {
        return Err(SCHEMA.into());
    }
    Ok(s.to_ascii_uppercase())
}

fn add(totals: &mut BTreeMap<String, f64>, unit: String, amount: f64) -> Result<(), String> {
    let total = totals.entry(unit).or_default();
    *total += amount;
    if total.is_finite() {
        Ok(())
    } else {
        Err(SCHEMA.into())
    }
}

fn sum_details(totals: BTreeMap<String, f64>, label: &str) -> Vec<AccountDetail> {
    if totals.is_empty() {
        return vec![detail(
            "report-status",
            "Report status",
            "No cost rows returned for this period",
            "status",
            None,
        )];
    }
    totals
        .into_iter()
        .map(|(unit, total)| detail(&format!("spend-{unit}"), label, total, "money", Some(&unit)))
        .collect()
}

fn period() -> (chrono::DateTime<Utc>, chrono::DateTime<Utc>) {
    let now = Utc::now();
    let first = now
        .date_naive()
        .with_day(1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    (first, now)
}

fn report_period(start: chrono::DateTime<Utc>, end: chrono::DateTime<Utc>) -> AccountDetail {
    detail(
        "report-period",
        "Report period (UTC)",
        format!(
            "{} to {}",
            start.to_rfc3339_opts(SecondsFormat::Secs, true),
            end.to_rfc3339_opts(SecondsFormat::Secs, true)
        ),
        "text",
        None,
    )
}

fn require_completed_period(
    start: chrono::DateTime<Utc>,
    end: chrono::DateTime<Utc>,
    bucket: &str,
) -> Result<(), String> {
    if end <= start {
        return Err(format!(
            "Billing temporarily unavailable: no completed UTC {bucket} this month. Credentials have not been validated. Try again after the next UTC {bucket}."
        ));
    }
    Ok(())
}

fn next_page(v: &Value, seen: &mut BTreeSet<String>) -> Result<Option<String>, String> {
    match v.get("has_more").and_then(Value::as_bool) {
        Some(false) => Ok(None),
        Some(true) => {
            let cursor = v
                .get("next_page")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 4096)
                .ok_or(SCHEMA)?;
            if !seen.insert(cursor.to_string()) {
                return Err(
                    "The report repeated a pagination cursor. No partial total was recorded."
                        .into(),
                );
            }
            Ok(Some(cursor.to_string()))
        }
        None => Err(SCHEMA.into()),
    }
}

// https://platform.openai.com/docs/api-reference/usage
// https://platform.claude.com/docs/en/manage-claude/usage-cost-api
// Anthropic's decimal strings are FRACTIONAL CENTS; OpenAI's amount.value is major units.
fn accumulate_costs(
    v: &Value,
    anthropic: bool,
    totals: &mut BTreeMap<String, f64>,
) -> Result<(), String> {
    for bucket in v.get("data").and_then(Value::as_array).ok_or(SCHEMA)? {
        for result in bucket
            .get("results")
            .and_then(Value::as_array)
            .ok_or(SCHEMA)?
        {
            let (unit, n) = if anthropic {
                (
                    currency(&result["currency"])?,
                    number(&result["amount"])? / 100.0,
                )
            } else {
                (
                    currency(&result["amount"]["currency"])?,
                    number(&result["amount"]["value"])?,
                )
            };
            add(totals, unit, n)?;
        }
    }
    Ok(())
}

async fn costs(
    client: &Client,
    credentials: &Credentials,
    anthropic: bool,
) -> Result<Vec<AccountDetail>, String> {
    let (start, now) = period();
    // Anthropic cost API returns complete daily buckets only. Show the exact cutoff.
    let end = if anthropic {
        now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc()
    } else {
        now
    };
    require_completed_period(start, end, "day")?;
    let url = if anthropic {
        "https://api.anthropic.com/v1/organizations/cost_report"
    } else {
        "https://api.openai.com/v1/organization/costs"
    };
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    let mut totals = BTreeMap::new();
    for _ in 0..MAX_PAGES {
        let mut req = client
            .get(url)
            .query(&[("limit", "31"), ("bucket_width", "1d")]);
        req = if anthropic {
            req.header("x-api-key", &credentials.access_token)
                .header("anthropic-version", "2023-06-01")
                .query(&[
                    ("starting_at", start.to_rfc3339()),
                    ("ending_at", end.to_rfc3339()),
                ])
        } else {
            req.bearer_auth(&credentials.access_token).query(&[
                ("start_time", start.timestamp()),
                ("end_time", end.timestamp()),
            ])
        };
        if let Some(ref page) = cursor {
            req = req.query(&[("page", page)]);
        }
        let v = json(req).await?;
        accumulate_costs(&v, anthropic, &mut totals)?;
        cursor = next_page(&v, &mut seen)?;
        if cursor.is_none() {
            let mut out = sum_details(
                totals,
                if anthropic {
                    "Reported API spend (completed days)"
                } else {
                    "Reported API spend this month"
                },
            );
            out.push(report_period(start, end));
            if anthropic {
                out.push(detail(
                    "coverage",
                    "Coverage",
                    "Priority Tier costs excluded by provider; billing data may lag",
                    "status",
                    None,
                ));
            }
            return Ok(out);
        }
    }
    Err("The cost report exceeded the pagination limit. No partial total was recorded.".into())
}

fn identifier(settings: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    let s = settings
        .get(key)
        .ok_or_else(|| format!("The {key} account setting is required."))?;
    if s.is_empty()
        || s.len() > 128
        || !s
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(format!("The {key} account setting is invalid."));
    }
    Ok(s.clone())
}

// https://docs.x.ai/developers/rest-api-reference/management/billing
fn xai_balance(v: &Value) -> Result<AccountDetail, String> {
    Ok(detail(
        "balance",
        "Reported prepaid ledger (signed)",
        number(&v["total"]["val"])? / 100.0,
        "money",
        Some("USD"),
    ))
}

async fn xai(
    client: &Client,
    credentials: &Credentials,
    settings: &BTreeMap<String, String>,
) -> Result<Vec<AccountDetail>, String> {
    let team = identifier(settings, "team_id")?;
    let base = format!("https://management-api.x.ai/v1/billing/teams/{team}");
    let balance = json(
        client
            .get(format!("{base}/prepaid/balance"))
            .bearer_auth(&credentials.access_token),
    )
    .await?;
    let mut out = vec![xai_balance(&balance)?];
    // Supplementary endpoint failure is explicit; retain the independently verified balance.
    match json(
        client
            .get(format!("{base}/postpaid/spending-limits"))
            .bearer_auth(&credentials.access_token),
    )
    .await
    {
        Ok(v) => {
            if let Ok(n) = number(&v["spendingLimits"]["effectiveSl"]["val"]) {
                out.push(detail(
                    "postpaid-cap",
                    "Effective postpaid monthly cap",
                    n / 100.0,
                    "money",
                    Some("USD"),
                ));
            } else {
                out.push(detail(
                    "cap-status",
                    "Postpaid cap",
                    "Not returned by provider",
                    "status",
                    None,
                ));
            }
        }
        Err(_) => out.push(detail(
            "cap-status",
            "Postpaid cap",
            "Unavailable: supplementary billing request failed",
            "status",
            None,
        )),
    }
    let (start, end) = period();
    let query = value!({"analyticsRequest": {
        "timeRange": {"startTime":start.format("%Y-%m-%d %H:%M:%S").to_string(), "endTime":end.format("%Y-%m-%d %H:%M:%S").to_string(), "timezone":"Etc/GMT"},
        "timeUnit":"TIME_UNIT_DAY", "values":[{"name":"usd","aggregation":"AGGREGATION_SUM"}],
        "groupBy":["description"], "filters":[]
    }});
    match json(
        client
            .post(format!("{base}/usage"))
            .bearer_auth(&credentials.access_token)
            .json(&query),
    )
    .await
    {
        Ok(v) => match xai_usage(&v) {
            Ok(n) => {
                out.push(detail(
                    "api-spend",
                    "Reported API spend this month",
                    n,
                    "money",
                    Some("USD"),
                ));
                out.push(report_period(start, end));
            }
            Err(_) => out.push(detail(
                "usage-status",
                "API spend",
                "Unavailable: usage response incomplete or truncated",
                "status",
                None,
            )),
        },
        Err(_) => out.push(detail(
            "usage-status",
            "API spend",
            "Unavailable: supplementary usage request failed",
            "status",
            None,
        )),
    }
    Ok(out)
}

fn xai_usage(v: &Value) -> Result<f64, String> {
    if v["limitReached"].as_bool() != Some(false) {
        return Err(SCHEMA.into());
    }
    let mut total = 0.0;
    for series in v["timeSeries"].as_array().ok_or(SCHEMA)? {
        for point in series["dataPoints"].as_array().ok_or(SCHEMA)? {
            let values = point["values"].as_array().ok_or(SCHEMA)?;
            if values.len() != 1 {
                return Err(SCHEMA.into());
            }
            total += number(&values[0])?;
        }
    }
    if total.is_finite() {
        Ok(total)
    } else {
        Err(SCHEMA.into())
    }
}

// https://docs.mistral.ai/api/endpoint/beta/admin/billing
fn mistral_limits(v: &Value) -> Result<Vec<AccountDetail>, String> {
    let reached = v
        .pointer("/limits/completion/monthly_limit_reached")
        .and_then(Value::as_bool)
        .ok_or(SCHEMA)?;
    let mut out = vec![detail(
        "monthly-cap-status",
        "API monthly limit",
        if reached { "Reached" } else { "Not reached" },
        "status",
        None,
    )];
    if let Some(failed) = v
        .pointer("/limits/last_payment_failure")
        .and_then(Value::as_bool)
    {
        out.push(detail(
            "payment-status",
            "Last payment",
            if failed {
                "Failed"
            } else {
                "No failure reported"
            },
            "status",
            None,
        ));
    }
    if let Some(no_limit) = v
        .pointer("/limits/completion/no_monthly_limit")
        .and_then(Value::as_bool)
    {
        out.push(detail(
            "cap-configured",
            "Monthly spend cap",
            if no_limit {
                "No monthly cap configured"
            } else {
                "Monthly cap configured"
            },
            "status",
            None,
        ));
    }
    let mut raw_reported = false;
    for (field, label) in [
        ("usage", "API spend (raw provider units)"),
        ("vibe_usage", "Vibe spend (raw provider units)"),
        ("total_usage", "Combined spend (raw provider units)"),
        ("usage_limit", "Spend cap (raw provider units)"),
    ] {
        if let Some(n) = v
            .pointer(&format!("/limits/completion/{field}"))
            .filter(|n| !n.is_null())
        {
            let unit = currency(&v["limits"]["currency"])?;
            out.push(detail(
                field,
                label,
                number(n)?,
                "usage",
                Some(&format!("{unit} provider units")),
            ));
            raw_reported = true;
        }
    }
    if raw_reported {
        out.push(detail("amount-units", "Amount units", "Provider schema does not specify major/minor currency scale; amounts shown without conversion", "status", None));
    }
    Ok(out)
}

async fn mistral(client: &Client, credentials: &Credentials) -> Result<Vec<AccountDetail>, String> {
    let v = json(
        client
            .get("https://api.mistral.ai/v1/admin/spend-limit")
            .bearer_auth(&credentials.access_token),
    )
    .await?;
    let mut out = mistral_limits(&v)?;
    match json(
        client
            .get("https://api.mistral.ai/v1/admin/rate-limit")
            .bearer_auth(&credentials.access_token),
    )
    .await
    {
        Ok(v) => {
            if let Ok(n) = number(&v["requests_per_second"]) {
                out.push(detail(
                    "requests-second",
                    "Configured throughput",
                    n,
                    "usage",
                    Some("requests/second"),
                ));
            } else {
                out.push(detail(
                    "rates-status",
                    "Rate limits",
                    "Provider did not return a usable throughput limit",
                    "status",
                    None,
                ));
            }
            if let Some(models) = v["tokens_limits_by_model"].as_object() {
                for (model, limits) in models {
                    for (field, suffix, unit) in [
                        (
                            "tokens_per_minute",
                            "configured token rate",
                            "tokens/minute",
                        ),
                        (
                            "tokens_per_month",
                            "configured monthly token limit",
                            "tokens/month",
                        ),
                    ] {
                        if let Some(raw) = limits.get(field).filter(|n| !n.is_null()) {
                            if let Ok(n) = number(raw) {
                                out.push(detail(
                                    &format!("{model}-{field}"),
                                    &format!("{model} {suffix}"),
                                    n,
                                    "usage",
                                    Some(unit),
                                ));
                            }
                        }
                    }
                }
            }
        }
        Err(_) => out.push(detail(
            "rates-status",
            "Rate limits",
            "Unavailable: supplementary rate-limit request failed",
            "status",
            None,
        )),
    }
    out.push(detail(
        "coverage",
        "Coverage",
        "Provider spend-limit report and configured rate; no consumer subscription renewal",
        "status",
        None,
    ));
    Ok(out)
}

// https://docs.perplexity.ai/docs/admin/computer-analytics-api
fn credit_count(v: &Value) -> Result<f64, String> {
    let mut total = 0.0;
    for bucket in v.get("data").and_then(Value::as_array).ok_or(SCHEMA)? {
        let n = number(&bucket["count"])?;
        if n < 0.0 {
            return Err(SCHEMA.into());
        }
        total += n;
    }
    if total.is_finite() {
        Ok(total)
    } else {
        Err(SCHEMA.into())
    }
}

async fn perplexity(
    client: &Client,
    credentials: &Credentials,
) -> Result<Vec<AccountDetail>, String> {
    let (start, now) = period();
    let end = chrono::DateTime::from_timestamp(now.timestamp() / 3600 * 3600, 0).ok_or(SCHEMA)?;
    require_completed_period(start, end, "hour")?;
    let mut total = 0.0;
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    for _ in 0..MAX_PAGES {
        let mut req = client
            .get("https://api.perplexity.ai/v1/analytics/computer/usage")
            .bearer_auth(&credentials.access_token)
            .query(&[
                ("dataset", "credit_usage"),
                ("bucket_width", "1h"),
                ("limit", "168"),
            ])
            .query(&[
                ("start_time", start.timestamp()),
                ("end_time", end.timestamp()),
            ]);
        if let Some(ref page) = cursor {
            req = req.query(&[("page", page)]);
        }
        let v = json(req).await?;
        total += credit_count(&v)?;
        if !total.is_finite() {
            return Err(SCHEMA.into());
        }
        cursor = next_page(&v, &mut seen)?;
        if cursor.is_none() {
            return Ok(vec![detail("credits-used", "Enterprise credits used (reported)", total, "usage", Some("credits")), report_period(start, end),
                detail("coverage", "Coverage", "Completed hours only; periodic sync can lag. This is usage, not remaining balance.", "status", None)]);
        }
    }
    Err("The analytics report exceeded the pagination limit. No partial total was recorded.".into())
}

// https://console.groq.com/docs/prometheus-metrics
fn prometheus_scalar(v: &Value) -> Result<Option<f64>, String> {
    if v["status"].as_str() != Some("success") || v["data"]["resultType"].as_str() != Some("vector")
    {
        return Err(SCHEMA.into());
    }
    let results = v["data"]["result"].as_array().ok_or(SCHEMA)?;
    if results.is_empty() {
        return Ok(None);
    }
    if results.len() != 1 {
        return Err(SCHEMA.into());
    }
    let n = number(&results[0]["value"][1])?;
    if n < 0.0 {
        return Err(SCHEMA.into());
    }
    Ok(Some(n))
}

async fn groq(client: &Client, credentials: &Credentials) -> Result<Vec<AccountDetail>, String> {
    let mut out = Vec::new();
    for (key, label, query) in [
        (
            "input-rate",
            "Input token rate (5-minute average)",
            "sum(model_project_id:tokens_in:rate5m)",
        ),
        (
            "output-rate",
            "Output token rate (5-minute average)",
            "sum(model_project_id:tokens_out:rate5m)",
        ),
    ] {
        let v = json(
            client
                .get("https://api.groq.com/v1/metrics/prometheus/api/v1/query")
                .bearer_auth(&credentials.access_token)
                .query(&[("query", query)]),
        )
        .await?;
        match prometheus_scalar(&v)? {
            Some(n) => out.push(detail(key, label, n, "usage", Some("tokens/second"))),
            None => out.push(detail(
                key,
                label,
                "No metric samples returned",
                "status",
                None,
            )),
        }
    }
    out.push(detail(
        "coverage",
        "Coverage",
        "Enterprise metrics; no account spend, balance or renewal is exposed here",
        "status",
        None,
    ));
    Ok(out)
}

// https://learn.microsoft.com/en-us/rest/api/cost-management/query/usage
fn azure_rows(v: &Value, totals: &mut BTreeMap<String, f64>) -> Result<(), String> {
    let p = v.get("properties").ok_or(SCHEMA)?;
    let columns = p["columns"].as_array().ok_or(SCHEMA)?;
    let money = columns
        .iter()
        .position(|c| matches!(c["name"].as_str(), Some("PreTaxCost") | Some("Cost")))
        .ok_or(SCHEMA)?;
    let unit = columns
        .iter()
        .position(|c| c["name"].as_str() == Some("Currency"))
        .ok_or(SCHEMA)?;
    for row in p["rows"].as_array().ok_or(SCHEMA)? {
        let cells = row.as_array().ok_or(SCHEMA)?;
        add(
            totals,
            currency(cells.get(unit).ok_or(SCHEMA)?)?,
            number(cells.get(money).ok_or(SCHEMA)?)?,
        )?;
    }
    Ok(())
}

fn azure_next(v: &Value, path: &str) -> Result<Option<String>, String> {
    let Some(next) = v.pointer("/properties/nextLink").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let next = next.as_str().ok_or(SCHEMA)?;
    if next.is_empty() {
        return Ok(None);
    }
    let url = reqwest::Url::parse(next).map_err(|_| SCHEMA)?;
    if url.scheme() != "https"
        || url.host_str() != Some("management.azure.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !url.path().eq_ignore_ascii_case(path)
    {
        return Err("The billing API returned an unsafe pagination link.".into());
    }
    Ok(Some(next.into()))
}

async fn azure(
    client: &Client,
    credentials: &Credentials,
    settings: &BTreeMap<String, String>,
) -> Result<Vec<AccountDetail>, String> {
    let subscription = identifier(settings, "subscription_id")?;
    if uuid::Uuid::parse_str(&subscription).is_err() {
        return Err("Azure subscription_id must be a UUID.".into());
    }
    let path = format!("/subscriptions/{subscription}/providers/Microsoft.CostManagement/query");
    let mut url = format!("https://management.azure.com{path}?api-version=2025-03-01");
    let body = value!({"type":"ActualCost", "timeframe":"MonthToDate", "dataset":{"granularity":"None", "aggregation":{"totalCost":{"name":"PreTaxCost","function":"Sum"}}}});
    let mut totals = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for _ in 0..MAX_PAGES {
        if !seen.insert(url.clone()) {
            return Err("The billing API repeated a page. No partial total was recorded.".into());
        }
        let v = json(
            client
                .post(&url)
                .bearer_auth(&credentials.access_token)
                .json(&body),
        )
        .await?;
        azure_rows(&v, &mut totals)?;
        match azure_next(&v, &path)? {
            Some(next) => url = next,
            None => {
                let mut out = sum_details(totals, "Azure subscription spend this month");
                out.push(detail(
                    "coverage",
                    "Coverage",
                    "All Azure services in this subscription; reported costs may lag",
                    "status",
                    None,
                ));
                if let Some(at) = credentials.expires_at {
                    out.push(detail(
                        "token-expiry",
                        "Reporting token expires",
                        at.to_rfc3339(),
                        "expiry",
                        None,
                    ));
                } else {
                    out.push(detail(
                        "token-expiry",
                        "Reporting credential",
                        "Short-lived Entra token; reconnect when expired",
                        "status",
                        None,
                    ));
                }
                return Ok(out);
            }
        }
    }
    Err("The billing report exceeded the pagination limit. No partial total was recorded.".into())
}

// BSS QueryAccountBalance, API version 2017-12-14:
// https://help.aliyun.com/en/user-center/developer-reference/api-queryaccountbalance
fn alibaba_balance(v: &Value) -> Result<Vec<AccountDetail>, String> {
    if v["Success"].as_bool() != Some(true) {
        return Err("Alibaba Cloud did not authorize the balance report. Check the CLI profile and billing read permissions.".into());
    }
    let unit = currency(&v["Data"]["Currency"])?;
    let mut out = vec![detail(
        "balance",
        "Alibaba account available balance",
        number(&v["Data"]["AvailableAmount"])?,
        "balance",
        Some(&unit),
    )];
    if let Some(n) = v
        .pointer("/Data/AvailableCashAmount")
        .filter(|v| !v.is_null())
    {
        out.push(detail(
            "cash-balance",
            "Cash balance",
            number(n)?,
            "balance",
            Some(&unit),
        ));
    }
    out.push(detail(
        "coverage",
        "Coverage",
        "Whole Alibaba Cloud account; not Model Studio free token quota",
        "status",
        None,
    ));
    Ok(out)
}

async fn alibaba(settings: &BTreeMap<String, String>) -> Result<Vec<AccountDetail>, String> {
    let profile = if settings.contains_key("profile") {
        Some(identifier(settings, "profile")?)
    } else {
        None
    };
    let region = settings
        .get("region")
        .map(String::as_str)
        .unwrap_or("cn-hangzhou");
    if !matches!(region, "cn-hangzhou" | "ap-southeast-1") {
        return Err("Choose cn-hangzhou or ap-southeast-1 for Alibaba billing.".into());
    }
    let region = region.to_string();
    let v = tokio::task::spawn_blocking(move || aliyun_report(profile, region))
        .await
        .map_err(|_| "The Alibaba billing reader stopped.".to_string())??;
    alibaba_balance(&v)
}

fn aliyun_report(profile: Option<String>, region: String) -> Result<Value, String> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        sync::mpsc,
        time::{Duration, Instant},
    };
    // Only this fixed read-only RPC is invoked. Credentials remain in the installed CLI profile.
    let binary = if cfg!(windows) {
        "aliyun.exe"
    } else {
        "aliyun"
    };
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.canonicalize().ok());
    let executable = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).filter(|p| p.is_absolute()).find_map(|directory| {
            let candidate = directory.join(binary).canonicalize().ok()?;
            if !candidate.is_file() || cwd.as_ref().is_some_and(|cwd| candidate.starts_with(cwd)) { return None; }
            Some(candidate)
        })
    }).ok_or("Alibaba billing requires an installed aliyun executable on PATH outside the working directory.")?;
    let mut command = Command::new(executable);
    command.args(["bssopenapi", "QueryAccountBalance", "--region", &region]);
    if let Some(profile) = profile {
        command.args(["--profile", &profile]);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().map_err(|_| {
        "Alibaba billing requires the already installed, authenticated aliyun CLI.".to_string()
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Could not read the Alibaba CLI report.")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes);
        let _ = tx.send((result.is_ok(), bytes));
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < Duration::from_secs(30) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("The Alibaba billing command timed out or could not complete.".into());
            }
        }
    };
    if !status.success() {
        return Err("Alibaba billing could not be read. Verify the existing CLI login and billing read permissions.".into());
    }
    let (ok, bytes) = rx
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| "The Alibaba billing output was interrupted.")?;
    if !ok || bytes.len() > 2 * 1024 * 1024 {
        return Err(
            "The Alibaba billing response exceeded the size limit or was interrupted.".into(),
        );
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "The Alibaba CLI returned an invalid billing report.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_reporting_period_cannot_validate_credentials() {
        let start = chrono::DateTime::<Utc>::from_timestamp(1_790_812_800, 0).unwrap();
        for bucket in ["day", "hour"] {
            let error = require_completed_period(start, start, bucket).unwrap_err();
            assert!(error.contains("Credentials have not been validated"));
            assert!(error.contains(bucket));
        }
        assert!(
            require_completed_period(start, start - chrono::Duration::seconds(1), "hour").is_err()
        );
        assert!(
            require_completed_period(start, start + chrono::Duration::hours(1), "hour").is_ok()
        );
    }

    #[test]
    fn cost_units_and_currencies_are_not_conflated() {
        let mut totals = BTreeMap::new();
        accumulate_costs(&value!({"data":[{"results":[{"amount":{"value":1.25,"currency":"usd"}},{"amount":{"value":2,"currency":"eur"}}]}]}), false, &mut totals).unwrap();
        accumulate_costs(
            &value!({"data":[{"results":[{"amount":"125.5","currency":"USD"}]}]}),
            true,
            &mut totals,
        )
        .unwrap();
        assert!((totals["USD"] - 2.505).abs() < 0.000_000_001);
        assert_eq!(totals["EUR"], 2.0);
    }

    #[test]
    fn missing_amount_or_currency_is_not_zero() {
        for v in [
            value!({}),
            value!({"data":null}),
            value!({"data":[{"results":[{"amount":{"value":null,"currency":"usd"}}]}]}),
            value!({"data":[{"results":[{"amount":{"value":1}}]}]}),
        ] {
            assert!(accumulate_costs(&v, false, &mut BTreeMap::new()).is_err());
        }
        assert!(number(&Value::Null).is_err());
        assert!(number(&value!("NaN")).is_err());
    }

    #[test]
    fn pagination_requires_explicit_end_and_fresh_cursor() {
        let mut seen = BTreeSet::new();
        assert!(next_page(&value!({}), &mut seen).is_err());
        assert!(next_page(&value!({"has_more":true,"next_page":null}), &mut seen).is_err());
        let v = value!({"has_more":true,"next_page":"a"});
        assert_eq!(next_page(&v, &mut seen).unwrap(), Some("a".into()));
        assert!(next_page(&v, &mut seen).is_err());
        assert_eq!(
            next_page(&value!({"has_more":false}), &mut seen).unwrap(),
            None
        );
    }

    #[test]
    fn xai_cents_allow_zero_and_negative_balance() {
        assert_eq!(
            xai_balance(&value!({"total":{"val":"1250"}}))
                .unwrap()
                .value,
            "12.5"
        );
        assert_eq!(
            xai_balance(&value!({"total":{"val":"0"}})).unwrap().value,
            "0"
        );
        assert_eq!(
            xai_balance(&value!({"total":{"val":"-25"}})).unwrap().value,
            "-0.25"
        );
        assert!(xai_balance(&value!({"total":null})).is_err());
    }

    #[test]
    fn xai_usage_refuses_truncated_or_unknown_coverage() {
        let report = value!({"limitReached":false,"timeSeries":[{"dataPoints":[{"values":[0.75]},{"values":[1.25]}]}]});
        assert_eq!(xai_usage(&report).unwrap(), 2.0);
        assert!(xai_usage(&value!({"limitReached":true,"timeSeries":[]})).is_err());
        assert!(xai_usage(&value!({"timeSeries":[]})).is_err());
        assert!(xai_usage(
            &value!({"limitReached":false,"timeSeries":[{"dataPoints":[{"values":[null]}]}]})
        )
        .is_err());
    }

    #[test]
    fn mistral_raw_amounts_do_not_guess_currency_scale() {
        let report = value!({"limits":{"currency":"EUR","completion":{"monthly_limit_reached":false,"usage":1234.5,"usage_limit":null}}});
        let details = mistral_limits(&report).unwrap();
        let usage = details.iter().find(|d| d.key == "usage").unwrap();
        assert_eq!(usage.value, "1234.5");
        assert_eq!(usage.unit.as_deref(), Some("EUR provider units"));
        assert!(!details.iter().any(|d| d.key == "usage_limit"));
    }

    #[test]
    fn explicit_mistral_false_is_valid_but_null_is_not() {
        assert_eq!(
            mistral_limits(&value!({"limits":{"completion":{"monthly_limit_reached":false}}}))
                .unwrap()[0]
                .value,
            "Not reached"
        );
        assert!(
            mistral_limits(&value!({"limits":{"completion":{"monthly_limit_reached":null}}}))
                .is_err()
        );
    }

    #[test]
    fn credit_usage_uses_total_not_overlapping_breakdowns() {
        assert_eq!(credit_count(&value!({"data":[{"count":8,"by_categories":{"Model":[{"count":8}],"Credit Source":[{"count":8}]}}]})).unwrap(), 8.0);
        assert!(credit_count(&value!({"data":[{"count":null}]})).is_err());
        assert!(credit_count(&value!({"data":[{"count":-1}]})).is_err());
    }

    #[test]
    fn metrics_distinguish_missing_samples_from_zero() {
        assert_eq!(
            prometheus_scalar(
                &value!({"status":"success","data":{"resultType":"vector","result":[]}})
            )
            .unwrap(),
            None
        );
        assert_eq!(prometheus_scalar(&value!({"status":"success","data":{"resultType":"vector","result":[{"value":[1,"0"]}]}})).unwrap(), Some(0.0));
        assert!(prometheus_scalar(&value!({"status":"success","data":{"resultType":"vector","result":[{"value":[1,"NaN"]}]}})).is_err());
    }

    #[test]
    fn azure_columns_are_located_by_name_and_links_stay_on_scope() {
        let mut totals = BTreeMap::new();
        azure_rows(&value!({"properties":{"columns":[{"name":"Currency"},{"name":"PreTaxCost"}],"rows":[["USD",3.5]]}}), &mut totals).unwrap();
        assert_eq!(totals["USD"], 3.5);
        assert!(azure_rows(
            &value!({"properties":{"columns":[{"name":"PreTaxCost"}],"rows":[[3.5]]}}),
            &mut totals
        )
        .is_err());
        let path = "/subscriptions/s/providers/Microsoft.CostManagement/query";
        assert!(azure_next(
            &value!({"properties":{"nextLink":"https://evil.example/bill"}}),
            path
        )
        .is_err());
        assert!(azure_next(&value!({"properties":{"nextLink":format!("https://management.azure.com{path}?$skiptoken=next")}}), path).unwrap().is_some());
    }

    #[test]
    fn alibaba_requires_success_currency_and_amount() {
        let v = value!({"Success":true,"Data":{"AvailableAmount":"1000.19","AvailableCashAmount":"0.00","Currency":"CNY"}});
        assert_eq!(alibaba_balance(&v).unwrap()[0].value, "1000.19");
        assert!(alibaba_balance(
            &value!({"Success":false,"Data":{"AvailableAmount":0,"Currency":"USD"}})
        )
        .is_err());
        assert!(alibaba_balance(
            &value!({"Success":true,"Data":{"AvailableAmount":null,"Currency":"USD"}})
        )
        .is_err());
        assert!(identifier(
            &BTreeMap::from([("team_id".into(), "../other".into())]),
            "team_id"
        )
        .is_err());
    }
}
