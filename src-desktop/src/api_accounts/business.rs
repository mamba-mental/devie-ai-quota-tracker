//! Read-only account adapters. Project only documented billing fields; never
//! retain raw user responses (Apify and Todoist can include credentials).
use super::helpers::{detail, json, params, reading};
use crate::{
    credentials::Credentials,
    model::{AccountDetail, QuotaAmount, QuotaReading, QuotaWindow},
};
use chrono::{Datelike, Utc};
use serde_json::Value;
use std::collections::BTreeMap;

const SHAPE: &str = "The provider did not return the documented account usage fields.";

pub async fn read(
    provider: &str,
    client: &reqwest::Client,
    credentials: &Credentials,
) -> Result<QuotaReading, String> {
    let p = params(credentials)?;
    if provider == "aws-billing" {
        let profile = parameter(&p, "profile")?.to_owned();
        validate_component(&profile)?;
        return tokio::task::spawn_blocking(move || aws_cli(&profile))
            .await
            .map_err(|_| "The AWS usage reader could not finish.".to_string())?;
    }
    if provider == "brave-search" {
        return Err("Brave does not have a confirmed account billing endpoint. No quota-consuming search was sent.".into());
    }
    if credentials.access_token.trim().is_empty() {
        return Err("An API token is required.".into());
    }
    let token = &credentials.access_token;
    match provider {
        "apify" => {
            let user = json(
                client
                    .get("https://api.apify.com/v2/users/me")
                    .bearer_auth(token),
            )
            .await?;
            let limits = json(
                client
                    .get("https://api.apify.com/v2/users/me/limits")
                    .bearer_auth(token),
            )
            .await?;
            apify(&user, &limits)
        }
        "firecrawl" => firecrawl(
            &json(
                client
                    .get("https://api.firecrawl.dev/v2/team/credit-usage")
                    .bearer_auth(token),
            )
            .await?,
        ),
        "hunter" => hunter(
            &json(
                client
                    .get("https://api.hunter.io/v2/account")
                    .query(&[("api_key", token)]),
            )
            .await?,
        ),
        "context7" => context7(
            &json(
                client
                    .get("https://context7.com/api/v2/usage")
                    .bearer_auth(token),
            )
            .await?,
        ),
        "cloudflare" => {
            let id = parameter(&p, "account_id")?;
            if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("Cloudflare requires a 32-character account ID.".into());
            }
            cloudflare(
                &json(
                    client
                        .get(format!(
                            "https://api.cloudflare.com/client/v4/accounts/{id}/subscriptions"
                        ))
                        .bearer_auth(token),
                )
                .await?,
            )
        }
        "make" => {
            let zone = parameter(&p, "zone")?;
            if !["eu1", "eu2", "us1", "us2"].contains(&zone) {
                return Err("Choose a supported Make zone: eu1, eu2, us1, or us2.".into());
            }
            let id = parameter(&p, "organization_id")?;
            if id.len() > 20 || !id.bytes().all(|b| b.is_ascii_digit()) {
                return Err("Make requires a numeric organization ID.".into());
            }
            let base = format!("https://{zone}.make.com/api/v2/organizations/{id}");
            let org = json(
                client
                    .get(&base)
                    .header("Authorization", format!("Token {token}")),
            )
            .await?;
            let usage = json(
                client
                    .get(format!("{base}/usage"))
                    .query(&[("organizationTimezone", "true")])
                    .header("Authorization", format!("Token {token}")),
            )
            .await?;
            make(&org, &usage)
        }
        "todoist" => todoist(
            &json(
                client
                    .get("https://api.todoist.com/api/v1/user")
                    .bearer_auth(token),
            )
            .await?,
        ),
        "github-billing" => {
            let username = parameter(&p, "username")?;
            if username.len() > 39
                || !username
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return Err("Enter a valid GitHub username.".into());
            }
            let now = Utc::now();
            github(
                &json(
                    client
                        .get(format!(
                            "https://api.github.com/users/{username}/settings/billing/usage"
                        ))
                        .bearer_auth(token)
                        .header("User-Agent", "Devie-Quota-Tracker")
                        .header("Accept", "application/vnd.github+json")
                        .header("X-GitHub-Api-Version", "2026-03-10")
                        .query(&[
                            ("year", now.year().to_string()),
                            ("month", now.month().to_string()),
                        ]),
                )
                .await?,
            )
        }
        _ => Err("This business account provider is not supported.".into()),
    }
}

fn parameter<'a>(p: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, String> {
    p.get(key)
        .map(String::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("The {key} field is required."))
}
fn validate_component(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || value.starts_with('-')
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_- .@".contains(&b))
    {
        return Err("Enter a valid AWS CLI profile name.".into());
    }
    Ok(())
}
fn check(v: &Value) -> Result<(), String> {
    if v.get("success") == Some(&Value::Bool(false))
        || v.get("error").is_some_and(|e| !e.is_null())
        || v.get("errors")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    {
        return Err("The provider rejected the account usage request.".into());
    }
    if !v.is_object() {
        return Err(SHAPE.into());
    }
    Ok(())
}
fn number(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite() && *n >= 0.0)
}
fn text(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| !s.trim().is_empty())
}
fn push(
    d: &mut Vec<AccountDetail>,
    key: &str,
    label: &str,
    v: &Value,
    kind: &str,
    unit: Option<&str>,
) {
    if let Some(s) = text(v) {
        d.push(detail(key, label, s, kind, unit));
    } else if let Some(n) = number(v) {
        d.push(detail(key, label, &n.to_string(), kind, unit));
    } else if let Some(b) = v.as_bool() {
        d.push(detail(key, label, if b { "Yes" } else { "No" }, kind, unit));
    }
}
fn finish(d: Vec<AccountDetail>) -> Result<QuotaReading, String> {
    if d.is_empty() {
        Err(SHAPE.into())
    } else {
        Ok(reading(d))
    }
}
fn window(
    key: &str,
    label: &str,
    used: &Value,
    total: &Value,
    unit: &str,
    reset: Option<&str>,
) -> Option<QuotaWindow> {
    let (used, total) = (number(used)?, number(total)?);
    if total <= 0.0 {
        return None;
    }
    let percent = used / total * 100.0;
    if !percent.is_finite() {
        return None;
    }
    Some(QuotaWindow {
        key: key.into(),
        label: label.into(),
        used_percent: percent.clamp(0.0, 100.0),
        resets_at: reset.map(str::to_owned),
        amount: Some(QuotaAmount {
            used: Some(used),
            total,
            unit: Some(unit.into()),
            overage: (used > total).then_some(used - total),
        }),
        ..Default::default()
    })
}

fn apify(user: &Value, limits: &Value) -> Result<QuotaReading, String> {
    check(user)?;
    check(limits)?;
    let u = &user["data"];
    let l = &limits["data"];
    let mut d = Vec::new();
    push(&mut d, "plan", "Plan", &u["plan"]["id"], "plan", None);
    push(
        &mut d,
        "included",
        "Monthly included usage",
        &u["plan"]["monthlyUsageCreditsUsd"],
        "money",
        Some("USD"),
    );
    push(
        &mut d,
        "used",
        "Cycle usage",
        &l["current"]["monthlyUsageUsd"],
        "usage",
        Some("USD"),
    );
    push(
        &mut d,
        "limit",
        "Monthly spending limit",
        &l["limits"]["maxMonthlyUsageUsd"],
        "money",
        Some("USD"),
    );
    push(
        &mut d,
        "reset",
        "Usage cycle ends",
        &l["monthlyUsageCycle"]["endAt"],
        "reset",
        None,
    );
    let mut r = finish(d)?;
    r.windows.extend(window(
        "spending",
        "Monthly spending limit",
        &l["current"]["monthlyUsageUsd"],
        &l["limits"]["maxMonthlyUsageUsd"],
        "USD",
        text(&l["monthlyUsageCycle"]["endAt"]),
    ));
    Ok(r)
}
fn firecrawl(v: &Value) -> Result<QuotaReading, String> {
    check(v)?;
    let v = &v["data"];
    let mut d = Vec::new();
    push(
        &mut d,
        "balance",
        "Remaining credits",
        &v["remainingCredits"],
        "balance",
        Some("credits"),
    );
    push(
        &mut d,
        "allocation",
        "Plan credits",
        &v["planCredits"],
        "usage",
        Some("credits"),
    );
    push(
        &mut d,
        "period_start",
        "Billing period starts",
        &v["billingPeriodStart"],
        "text",
        None,
    );
    push(
        &mut d,
        "period_end",
        "Billing period ends",
        &v["billingPeriodEnd"],
        "text",
        None,
    );
    // Extra credits can exceed planCredits: never derive usage by subtraction.
    finish(d)
}
fn hunter(v: &Value) -> Result<QuotaReading, String> {
    check(v)?;
    let v = &v["data"];
    let mut d = Vec::new();
    let mut windows = Vec::new();
    push(&mut d, "plan", "Plan", &v["plan_name"], "plan", None);
    push(
        &mut d,
        "reset",
        "Requests reset",
        &v["reset_date"],
        "reset",
        None,
    );
    for bucket in ["credits", "searches", "verifications"] {
        let b = &v["requests"][bucket];
        for (field, label, kind) in [
            ("remaining", "remaining", "balance"),
            ("used", "used", "usage"),
            ("available", "allocation", "usage"),
        ] {
            push(
                &mut d,
                &format!("{bucket}_{field}"),
                &format!("{bucket} {label}"),
                &b[field],
                kind,
                Some(bucket),
            );
        }
        windows.extend(window(
            bucket,
            bucket,
            &b["used"],
            &b["available"],
            bucket,
            text(&v["reset_date"]),
        ));
    }
    let mut r = finish(d)?;
    r.windows = windows;
    Ok(r)
}
fn context7(v: &Value) -> Result<QuotaReading, String> {
    check(v)?;
    let mut d = Vec::new();
    for (key, label, value, kind, unit) in [
        ("plan", "Plan", &v["plan"], "plan", None),
        (
            "member_requests",
            "Member requests this month",
            &v["member"]["requests"],
            "usage",
            Some("requests"),
        ),
        (
            "included",
            "Included member requests",
            &v["member"]["includedInPlan"],
            "usage",
            Some("requests"),
        ),
        (
            "overage",
            "Team extra request cost",
            &v["teamspace"]["extraCostUsd"],
            "money",
            Some("USD"),
        ),
        (
            "spend_limit",
            "Team extra request spending limit",
            &v["teamspace"]["spendingLimitUsd"],
            "money",
            Some("USD"),
        ),
        ("reset", "Usage resets", &v["usageResetsAt"], "reset", None),
    ] {
        push(&mut d, key, label, value, kind, unit);
    }
    let mut r = finish(d)?;
    r.windows.extend(window(
        "requests",
        "Member included requests",
        &v["member"]["requests"],
        &v["member"]["includedInPlan"],
        "requests",
        text(&v["usageResetsAt"]),
    ));
    Ok(r)
}
fn cloudflare(v: &Value) -> Result<QuotaReading, String> {
    check(v)?;
    let rows = v["result"].as_array().ok_or(SHAPE)?;
    let mut d = Vec::new();
    if rows.is_empty() {
        d.push(detail(
            "subscriptions",
            "Account subscriptions returned",
            "0",
            "status",
            None,
        ));
    }
    for (i, row) in rows.iter().enumerate() {
        for (field, label, kind) in [
            ("state", "Subscription status", "status"),
            ("frequency", "Billing frequency", "text"),
            ("current_period_end", "Next billing due", "renewal"),
        ] {
            push(
                &mut d,
                &format!("{i}_{field}"),
                &format!("{}: {label}", i + 1),
                &row[field],
                kind,
                None,
            );
        }
        push(
            &mut d,
            &format!("{i}_plan"),
            &format!("{}: Plan", i + 1),
            &row["rate_plan"]["public_name"],
            "plan",
            None,
        );
        push(
            &mut d,
            &format!("{i}_price"),
            &format!("{}: Subscription price", i + 1),
            &row["price"],
            "money",
            text(&row["currency"]),
        );
    }
    finish(d)
}
fn make(org: &Value, usage: &Value) -> Result<QuotaReading, String> {
    check(org)?;
    check(usage)?;
    let mut d = Vec::new();
    let o = &org["organization"];
    push(
        &mut d,
        "plan",
        "Service plan",
        &o["serviceName"],
        "plan",
        None,
    );
    push(
        &mut d,
        "reset",
        "Usage resets",
        &o["nextReset"],
        "reset",
        None,
    );
    let rows = usage["data"].as_array().ok_or(SHAPE)?;
    for (key, label, unit) in [
        (
            "operations",
            "Operations in returned 30-day history",
            "operations",
        ),
        (
            "centicredits",
            "Centicredits in returned 30-day history",
            "centicredits",
        ),
        (
            "dataTransfer",
            "Data transfer in returned 30-day history",
            "bytes",
        ),
    ] {
        if !rows.is_empty() && rows.iter().all(|r| number(&r[key]).is_some()) {
            let sum: f64 = rows.iter().filter_map(|r| number(&r[key])).sum();
            if sum.is_finite() {
                d.push(detail(key, label, &sum.to_string(), "usage", Some(unit)));
            }
        }
    }
    if rows.is_empty() {
        d.push(detail(
            "history",
            "Returned daily usage records",
            "0",
            "status",
            None,
        ));
    }
    // Rolling 30-day usage is not billing-cycle usage and has no known denominator.
    finish(d)
}
fn todoist(v: &Value) -> Result<QuotaReading, String> {
    check(v)?;
    let mut d = Vec::new();
    push(
        &mut d,
        "premium",
        "Pro entitlement active",
        &v["is_premium"],
        "status",
        None,
    );
    push(
        &mut d,
        "plan",
        "Subscription status",
        &v["premium_status"],
        "plan",
        None,
    );
    push(
        &mut d,
        "expiry",
        "Pro expires (excluding grace period)",
        &v["premium_until"],
        "expiry",
        None,
    );
    finish(d)
}
fn github(v: &Value) -> Result<QuotaReading, String> {
    check(v)?;
    let rows = v["usageItems"].as_array().ok_or(SHAPE)?;
    if rows.iter().any(|r| number(&r["netAmount"]).is_none()) {
        return Err(SHAPE.into());
    }
    let total: f64 = rows.iter().filter_map(|r| number(&r["netAmount"])).sum();
    if !total.is_finite() {
        return Err(SHAPE.into());
    }
    finish(vec![
        detail(
            "net_cost",
            "Current month net billed usage",
            &total.to_string(),
            "money",
            Some("USD"),
        ),
        detail(
            "scope",
            "Billing scope",
            "Personal account; excludes organization-billed licenses",
            "text",
            None,
        ),
    ])
}

fn aws_cli(profile: &str) -> Result<QuotaReading, String> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut command = Command::new("aws");
    command
        .args([
            "freetier",
            "get-free-tier-usage",
            "--output",
            "json",
            "--profile",
            profile,
            "--no-cli-pager",
            "--no-paginate",
            "--max-results",
            "1000",
        ])
        .env("AWS_PAGER", "")
        .env("AWS_CLI_AUTO_PROMPT", "off")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|_| "AWS CLI could not start. Install/configure it separately and select an existing profile.".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or("AWS CLI output was unavailable.")?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Ok(s),
            Ok(None) if started.elapsed() < Duration::from_secs(30) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("AWS CLI usage request timed out or failed.".to_string());
            }
        }
    };
    let bytes = receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| "AWS output reader did not finish.".to_string())?
        .map_err(|_| "AWS output could not be read.".to_string())?;
    if !status?.success() {
        return Err("AWS CLI denied the usage request. Check the existing profile and freetier:GetFreeTierUsage permission.".into());
    }
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("AWS usage response exceeded the size limit.".into());
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "AWS returned invalid usage JSON.".to_string())?;
    aws(&value)
}
fn aws(v: &Value) -> Result<QuotaReading, String> {
    check(v)?;
    if text(&v["nextToken"]).is_some() {
        return Err(
            "AWS returned more than 1000 Free Tier records; this partial result was not saved."
                .into(),
        );
    }
    let rows = v["freeTierUsages"].as_array().ok_or(SHAPE)?;
    let mut d = vec![detail(
        "scope",
        "Report scope",
        "AWS Free Tier allowances only; excludes total bill and promotional-credit balance",
        "text",
        None,
    )];
    let mut windows = Vec::new();
    if rows.is_empty() {
        d.push(detail(
            "records",
            "Free Tier allowances returned",
            "0",
            "status",
            None,
        ));
    }
    for (i, row) in rows.iter().enumerate() {
        let label = format!(
            "{} / {} / {}",
            text(&row["service"]).unwrap_or("Service"),
            text(&row["usageType"]).unwrap_or("Usage"),
            text(&row["region"]).unwrap_or("Global")
        );
        let unit = text(&row["unit"]).unwrap_or("units");
        if number(&row["actualUsageAmount"]).is_none() || number(&row["limit"]).is_none() {
            return Err(SHAPE.into());
        }
        push(
            &mut d,
            &format!("{i}_used"),
            &format!("{label}: used"),
            &row["actualUsageAmount"],
            "usage",
            Some(unit),
        );
        push(
            &mut d,
            &format!("{i}_limit"),
            &format!("{label}: allowance"),
            &row["limit"],
            "usage",
            Some(unit),
        );
        windows.extend(window(
            &format!("freetier_{i}"),
            &label,
            &row["actualUsageAmount"],
            &row["limit"],
            unit,
            None,
        ));
    }
    let mut r = finish(d)?;
    r.windows = windows;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn hunter_uses_allocation_as_denominator() {
        let r = hunter(
            &json!({"data":{"requests":{"credits":{"used":25,"available":100,"remaining":75}}}}),
        )
        .unwrap();
        assert_eq!(r.windows[0].used_percent, 25.0);
        assert_eq!(r.windows[0].amount.as_ref().unwrap().total, 100.0);
    }
    #[test]
    fn missing_and_error_payloads_are_not_zero_usage() {
        for v in [
            json!({}),
            json!({"data":null}),
            json!({"error":"secret upstream error"}),
            json!({"success":false}),
        ] {
            assert!(firecrawl(&v).is_err());
            assert!(hunter(&v).is_err());
            assert!(todoist(&v).is_err());
        }
        assert!(github(&json!({"usageItems":[{"netAmount":null}]})).is_err());
        assert!(aws(&json!({"freeTierUsages":[{}]})).is_err());
    }
    #[test]
    fn firecrawl_extra_credits_have_no_invented_percent() {
        let r = firecrawl(&json!({"data":{"remainingCredits":150,"planCredits":100}})).unwrap();
        assert!(r.windows.is_empty());
    }
    #[test]
    fn allowlists_exclude_secrets() {
        let r = apify(
            &json!({"data":{"plan":{"id":"Free"},"proxy":{"password":"never-store"}}}),
            &json!({"data":{"current":{"monthlyUsageUsd":5},"limits":{"maxMonthlyUsageUsd":10}}}),
        )
        .unwrap();
        assert_eq!(r.windows[0].used_percent, 50.0);
        assert!(!serde_json::to_string(&r.identity)
            .unwrap()
            .contains("never-store"));
        let t = todoist(&json!({"is_premium":false,"premium_until":null,"token":"never-store"}))
            .unwrap();
        assert!(!serde_json::to_string(&t.identity)
            .unwrap()
            .contains("never-store"));
    }
    #[test]
    fn zero_caps_and_unknown_reset_are_not_fabricated() {
        let r=context7(&json!({"plan":"payg","member":{"requests":12,"includedInPlan":0},"teamspace":{"spendingLimitUsd":null}})).unwrap();
        assert!(r.windows.is_empty());
        assert!(window("x", "x", &json!(-1), &json!(10), "units", None).is_none());
        assert!(window("x", "x", &json!(1), &Value::Null, "units", None).is_none());
    }
    #[test]
    fn subscription_dates_are_not_quota_windows() {
        let r=cloudflare(&json!({"success":true,"result":[{"state":"Paid","current_period_end":"2026-11-01T00:00:00Z"}]})).unwrap();
        assert!(r.windows.is_empty());
        assert!(cloudflare(&json!({"success":false,"errors":[{"message":"private"}]})).is_err());
    }
    #[test]
    fn usage_readers_require_known_fields() {
        assert!(make(&json!({}), &json!({"data":[{}]})).is_err());
        assert!(make(
            &json!({}),
            &json!({"data":[{"operations":2,"centicredits":300,"dataTransfer":4}]})
        )
        .is_ok());
        assert!(github(&json!({"usageItems":[]})).is_ok());
        let r=aws(&json!({"freeTierUsages":[{"service":"S3","actualUsageAmount":2,"limit":5,"unit":"GB"}]})).unwrap();
        assert_eq!(r.windows[0].used_percent, 40.0);
        assert!(aws(&json!({"freeTierUsages":[],"nextToken":"more"})).is_err());
    }
    #[test]
    fn profile_is_data_not_command_options() {
        assert!(validate_component("work-profile").is_ok());
        for s in ["--endpoint-url", "a\nb", "x; touch file", "$(command)", ""] {
            assert!(validate_component(s).is_err());
        }
    }
    #[tokio::test]
    async fn unsupported_and_missing_auth_do_not_send_requests() {
        let c = reqwest::Client::new();
        let creds = Credentials::default();
        assert!(read("brave-search", &c, &creds)
            .await
            .unwrap_err()
            .contains("No quota-consuming"));
        assert!(read("hunter", &c, &creds)
            .await
            .unwrap_err()
            .contains("token"));
    }
}
