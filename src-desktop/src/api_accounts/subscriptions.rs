//! Read-only subscription/balance readers. Never infer a plan from model access.
//! Contracts: Chutes api/user/router.py; zai-org/zai-coding-plugins usage-query;
//! MiniMax-AI/cli src/output/quota-table.ts; MoonshotAI/kimi-code server-api.md;
//! creator.poe.com/docs/resources/usage-api; platform.kimi.ai/docs/api/balance.

use super::helpers::{detail, json, params, reading};
use crate::{
    credentials::Credentials,
    model::{QuotaAmount, QuotaReading, QuotaWindow},
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde_json::Value;

const INVALID: &str = "The account response is missing supported quota or balance data.";
const REJECTED: &str =
    "The provider rejected the account request. Check the account login and permissions.";

pub async fn read(
    provider: &str,
    client: &Client,
    credentials: &Credentials,
) -> Result<QuotaReading, String> {
    if credentials.access_token.trim().is_empty() {
        return Err("An account API key or local server token is required.".into());
    }
    let fields = params(credentials)?;
    let key = &credentials.access_token;
    match provider {
        "chutes" => parse_chutes(
            &json(
                client
                    .get("https://api.chutes.ai/users/me/subscription_usage")
                    .bearer_auth(key),
            )
            .await?,
        ),
        "minimax" => {
            let (url, balance) = minimax_endpoint(key);
            let value = json(
                client
                    .get(url)
                    .bearer_auth(key)
                    .header("Content-Type", "application/json"),
            )
            .await?;
            if balance {
                parse_minimax_balance(&value)
            } else {
                parse_minimax(&value)
            }
        }
        "zai" => {
            let host = match fields.get("region").map(String::as_str).unwrap_or("global") {
                "global" => "https://api.z.ai",
                "cn" => "https://open.bigmodel.cn",
                _ => return Err("Choose the global or cn account region.".into()),
            };
            // The official usage plugin uses a raw Authorization token.
            parse_zai(
                &json(
                    client
                        .get(format!("{host}/api/monitor/usage/quota/limit"))
                        .header("Authorization", key)
                        .header("Accept-Language", "en-US,en"),
                )
                .await?,
            )
        }
        "poe" => parse_poe(
            &json(
                client
                    .get("https://api.poe.com/usage/current_balance")
                    .bearer_auth(key),
            )
            .await?,
        ),
        "moonshot" => {
            let (url, unit) = match fields.get("region").map(String::as_str).unwrap_or("global") {
                "global" => ("https://api.moonshot.ai/v1/users/me/balance", "USD"),
                "cn" => ("https://api.moonshot.cn/v1/users/me/balance", "CNY"),
                _ => return Err("Choose the global or cn account region.".into()),
            };
            parse_moonshot(&json(client.get(url).bearer_auth(key)).await?, unit)
        }
        "kimi-code" => {
            let port = local_port(fields.get("port").map(String::as_str))?;
            let base = format!("http://127.0.0.1:{port}/api/v1/oauth");
            let mut result =
                parse_kimi(&json(client.get(format!("{base}/usage")).bearer_auth(key)).await?)?;
            // Profile metadata is optional; a failed profile read must not erase quotas.
            if let Ok(profile) = json(client.get(format!("{base}/userinfo")).bearer_auth(key)).await
            {
                if profile["code"].as_i64() == Some(0) && profile["data"]["kind"] == "ok" {
                    if let Some(plan) = short_text(&profile["data"]["userInfo"]["userLevelName"]) {
                        if let Some(identity) = result.identity.as_mut() {
                            identity.plan = Some(plan);
                        }
                    }
                }
            }
            Ok(result)
        }
        _ => Err("This subscription reader is not available.".into()),
    }
}

fn local_port(value: Option<&str>) -> Result<u16, String> {
    match value {
        None | Some("") => Ok(58627),
        Some(value) if value.bytes().all(|b| b.is_ascii_digit()) => value
            .parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
            .ok_or_else(|| "The local Kimi server port must be between 1 and 65535.".into()),
        _ => Err("The local Kimi server port must be between 1 and 65535.".into()),
    }
}

fn minimax_endpoint(key: &str) -> (&'static str, bool) {
    if key.starts_with("sk-api-") {
        ("https://api.minimax.io/account/query_balance", true)
    } else {
        ("https://www.minimax.io/v1/token_plan/remains", false)
    }
}

fn parse_minimax_balance(v: &Value) -> Result<QuotaReading, String> {
    no_api_error(v)?;
    if v["base_resp"]["status_code"].as_i64() != Some(0) {
        return Err(REJECTED.into());
    }
    let mut details = Vec::new();
    for (key, label) in [
        ("available_amount", "Available balance"),
        ("cash_balance", "Cash balance"),
        ("voucher_balance", "Voucher balance"),
        ("credit_balance", "Credit balance"),
        ("owed_amount", "Amount owed"),
    ] {
        let value = decimal_string(&v[key])?;
        details.push(detail(key, label, value, "balance", None));
    }
    details.push(detail(
        "currency",
        "Balance currency",
        "Not reported by this API",
        "status",
        None,
    ));
    let alert = v["balance_alert_switch"].as_bool().ok_or(INVALID)?;
    details.push(detail(
        "balance_alert_switch",
        "Balance alerts",
        if alert { "Enabled" } else { "Disabled" },
        "status",
        None,
    ));
    if let Some(value) = v.get("balance_alert_threshold") {
        if value.as_str() != Some("") {
            details.push(detail(
                "balance_alert_threshold",
                "Balance alert threshold",
                decimal_string(value)?,
                "limit",
                None,
            ));
        }
    }
    Ok(reading(details))
}

fn decimal_string(v: &Value) -> Result<&str, String> {
    let value = v
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 64 && s.trim() == *s)
        .ok_or(INVALID)?;
    value
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .ok_or(INVALID)?;
    Ok(value)
}

fn number(v: &Value) -> Result<f64, String> {
    v.as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| INVALID.into())
}
fn nonnegative(v: &Value) -> Result<f64, String> {
    let n = number(v)?;
    if n < 0.0 {
        Err(INVALID.into())
    } else {
        Ok(n)
    }
}
fn percentage(v: &Value) -> Result<f64, String> {
    let n = nonnegative(v)?;
    if n > 100.0 {
        Err(INVALID.into())
    } else {
        Ok(n)
    }
}
fn short_text(v: &Value) -> Option<String> {
    v.as_str()
        .filter(|s| {
            !s.trim().is_empty() && s.chars().count() <= 120 && !s.chars().any(char::is_control)
        })
        .map(str::to_owned)
}
fn date(v: &Value) -> Result<Option<String>, String> {
    if v.is_null() {
        return Ok(None);
    }
    let value = v.as_str().ok_or(INVALID)?;
    DateTime::parse_from_rfc3339(value)
        .map(|d| Some(d.to_rfc3339()))
        .map_err(|_| INVALID.into())
}
fn millis_date(v: &Value) -> Result<Option<String>, String> {
    if v.is_null() {
        return Ok(None);
    }
    let ms = v.as_i64().filter(|n| *n > 0).ok_or(INVALID)?;
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|d| Some(d.to_rfc3339()))
        .ok_or_else(|| INVALID.into())
}
fn window(key: &str, label: &str, percent: f64, reset: Option<String>) -> QuotaWindow {
    QuotaWindow {
        key: key.into(),
        label: label.into(),
        used_percent: percent,
        resets_at: reset,
        ..Default::default()
    }
}
fn no_api_error(v: &Value) -> Result<(), String> {
    if !v.is_object() {
        return Err(INVALID.into());
    }
    if v.get("error").is_some_and(|e| !e.is_null()) {
        return Err(REJECTED.into());
    }
    Ok(())
}

fn parse_poe(v: &Value) -> Result<QuotaReading, String> {
    no_api_error(v)?;
    let balance = v["current_point_balance"].as_u64().ok_or(INVALID)?;
    Ok(reading(vec![detail(
        "point_balance",
        "Available plan and add-on points",
        balance,
        "balance",
        Some("points"),
    )]))
}

fn parse_moonshot(v: &Value, currency: &str) -> Result<QuotaReading, String> {
    no_api_error(v)?;
    if v["code"].as_i64() != Some(0) || v["status"].as_bool() != Some(true) {
        return Err(REJECTED.into());
    }
    let d = &v["data"];
    let available = number(&d["available_balance"])?;
    let cash = number(&d["cash_balance"])?;
    let voucher = nonnegative(&d["voucher_balance"])?;
    Ok(reading(vec![
        detail(
            "available_balance",
            "Available balance",
            available,
            "balance",
            Some(currency),
        ),
        detail(
            "cash_balance",
            "Cash balance",
            cash,
            "balance",
            Some(currency),
        ),
        detail(
            "voucher_balance",
            "Voucher balance",
            voucher,
            "balance",
            Some(currency),
        ),
    ]))
}

fn parse_chutes(v: &Value) -> Result<QuotaReading, String> {
    no_api_error(v)?;
    let subscribed = v["subscription"].as_bool().ok_or(INVALID)?;
    let mut result = reading(vec![detail(
        "subscription",
        "Subscription",
        if subscribed {
            "Active"
        } else {
            "No active subscription"
        },
        "status",
        None,
    )]);
    if !subscribed {
        return Ok(result);
    }
    let custom = v["custom"].as_bool().ok_or(INVALID)?;
    for (key, label) in [
        ("four_hour", "Four-hour allowance"),
        ("monthly", "Monthly allowance"),
    ] {
        let entry = &v[key];
        if key == "monthly" && custom && entry["uncapped"].as_bool() == Some(true) {
            result.windows.push(QuotaWindow {
                unlimited: true,
                ..window(key, label, 0.0, None)
            });
            continue;
        }
        let used = nonnegative(&entry["usage"])?;
        let cap = nonnegative(&entry["cap"])?;
        let remaining = nonnegative(&entry["remaining"])?;
        if cap <= 0.0 || remaining > cap {
            return Err(INVALID.into());
        }
        let mut w = window(
            key,
            label,
            (used / cap * 100.0).clamp(0.0, 100.0),
            date(&entry["reset_at"])?,
        );
        w.amount = Some(QuotaAmount {
            used: Some(used),
            total: cap,
            unit: Some("USD".into()),
            overage: None,
        });
        result.windows.push(w);
    }
    if let Some(identity) = result.identity.as_mut() {
        identity.details.push(detail(
            "monthly_price",
            "Monthly subscription price",
            nonnegative(&v["monthly_price"])?,
            "tier",
            Some("USD"),
        ));
        for (key, label) in [
            ("anchor_date", "Quota cycle anchor"),
            ("effective_date", "Quota effective date"),
        ] {
            if let Some(value) = date(&v[key])? {
                identity
                    .details
                    .push(detail(key, label, value, "status", None));
            }
        }
    }
    Ok(result)
}

fn parse_zai(v: &Value) -> Result<QuotaReading, String> {
    no_api_error(v)?;
    if v.get("success").is_some_and(|s| s.as_bool() != Some(true))
        || v.get("code")
            .is_some_and(|c| !matches!(c.as_i64(), Some(0) | Some(200)))
    {
        return Err(REJECTED.into());
    }
    let limits = v["data"]["limits"]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 100)
        .ok_or(INVALID)?;
    let mut result = reading(vec![]);
    for (index, entry) in limits.iter().enumerate() {
        let (key, label) = match entry["type"].as_str() {
            Some("TOKENS_LIMIT") => ("tokens", "Five-hour token allowance"),
            Some("TIME_LIMIT") => ("mcp", "Monthly MCP allowance"),
            // CREDIT_LIMIT has no official window mapping; preserve its value as a detail.
            Some("CREDIT_LIMIT") => {
                let percent = percentage(&entry["percentage"])?;
                if let Some(identity) = result.identity.as_mut() {
                    identity.details.push(detail(
                        &format!("credit_limit_{index}"),
                        "Coding quota reported percentage (window unspecified)",
                        percent,
                        "usage",
                        Some("%"),
                    ));
                }
                continue;
            }
            _ => continue,
        };
        let percent = percentage(&entry["percentage"])?;
        let mut w = window(&format!("{key}_{index}"), label, percent, None);
        if key == "mcp" && (!entry["currentValue"].is_null() || !entry["usage"].is_null()) {
            let used = nonnegative(&entry["currentValue"])?;
            let total = nonnegative(&entry["usage"])?;
            if total <= 0.0 {
                return Err(INVALID.into());
            }
            w.amount = Some(QuotaAmount {
                used: Some(used),
                total,
                unit: Some("calls".into()),
                overage: None,
            });
        }
        result.windows.push(w);
    }
    if result.windows.is_empty()
        && result
            .identity
            .as_ref()
            .map_or(true, |i| i.details.is_empty())
    {
        return Err(
            "Z.ai returned quota types whose window semantics are not yet supported.".into(),
        );
    }
    Ok(result)
}

fn parse_minimax(v: &Value) -> Result<QuotaReading, String> {
    no_api_error(v)?;
    if v["base_resp"]["status_code"].as_i64() != Some(0) {
        return Err(REJECTED.into());
    }
    let models = v["model_remains"]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 100)
        .ok_or(INVALID)?;
    let mut result = reading(vec![]);
    for (index, entry) in models.iter().enumerate() {
        let model = short_text(&entry["model_name"]).ok_or(INVALID)?;
        // Official CLI: both status=3 with zero totals means NOT in the plan.
        if entry["current_interval_status"].as_i64() == Some(3)
            && entry["current_weekly_status"].as_i64() == Some(3)
            && entry["current_interval_total_count"].as_f64() == Some(0.0)
            && entry["current_weekly_total_count"].as_f64() == Some(0.0)
        {
            if let Some(i) = result.identity.as_mut() {
                i.details.push(detail(
                    &format!("model_{index}"),
                    &model,
                    "Not included in this plan",
                    "status",
                    None,
                ));
            }
            continue;
        }
        for (prefix, suffix, label, reset_key) in [
            ("current_interval", "interval", "Current window", "end_time"),
            ("current_weekly", "weekly", "Weekly", "weekly_end_time"),
        ] {
            let status = &entry[format!("{prefix}_status")];
            if !status.is_null() && !matches!(status.as_i64(), Some(1..=3)) {
                return Err(INVALID.into());
            }
            let id = format!("model_{index}_{suffix}");
            let label = format!("{model} · {label}");
            let reset = millis_date(&entry[reset_key])?;
            if suffix == "weekly" && status.as_i64() == Some(3) {
                result.windows.push(QuotaWindow {
                    unlimited: true,
                    ..window(&id, &label, 0.0, reset)
                });
                continue;
            }
            let remaining_value = &entry[format!("{prefix}_remaining_percent")];
            if remaining_value.is_null() {
                continue;
            } // Never infer semantics of legacy usage_count.
            let remaining = percentage(remaining_value)?;
            let boost = if suffix == "weekly" && !entry["weekly_boost_permille"].is_null() {
                nonnegative(&entry["weekly_boost_permille"])? / 1000.0
            } else {
                1.0
            };
            if boost != 1.0 {
                // Boosted remaining can exceed 100%; keep the reported headroom as a detail.
                let boosted = remaining * boost;
                if !boosted.is_finite() {
                    return Err(INVALID.into());
                }
                if let Some(i) = result.identity.as_mut() {
                    i.details.push(detail(
                        &id,
                        &format!("{label} remaining (boosted)"),
                        boosted,
                        "balance",
                        Some("%"),
                    ));
                    if let Some(reset) = reset {
                        i.details.push(detail(
                            &format!("{id}_reset"),
                            &format!("{label} quota reset"),
                            reset,
                            "status",
                            None,
                        ));
                    }
                }
            } else {
                result
                    .windows
                    .push(window(&id, &label, 100.0 - remaining, reset));
            }
        }
    }
    if result.windows.is_empty()
        && result
            .identity
            .as_ref()
            .map_or(true, |i| i.details.is_empty())
    {
        return Err("MiniMax did not return supported remaining percentages. Legacy count semantics cannot be safely inferred.".into());
    }
    Ok(result)
}

fn parse_kimi(v: &Value) -> Result<QuotaReading, String> {
    no_api_error(v)?;
    if v["code"].as_i64() != Some(0) || v["data"]["kind"].as_str() != Some("ok") {
        return Err("Kimi Code usage is unavailable. Start its local server and sign in to the managed Kimi account.".into());
    }
    let quota = &v["data"]["quota"];
    let usages = quota["usages"].as_object().ok_or(INVALID)?;
    let mut result = reading(vec![]);
    for (key, label) in [
        ("limit5h", "Five-hour allowance"),
        ("limit7d", "Weekly allowance"),
        ("monthTotal", "Monthly total allowance"),
        ("monthCode", "Monthly coding allowance"),
    ] {
        let Some(entry) = usages.get(key) else {
            continue;
        };
        let ratio = nonnegative(&entry["usedRatio"])?;
        if ratio > 1.0 {
            return Err(INVALID.into());
        }
        result
            .windows
            .push(window(key, label, ratio * 100.0, date(&entry["resetAt"])?));
    }
    if let Some(wallet) = quota.get("extraUsage").filter(|w| !w.is_null()) {
        let currency = wallet["currency"]
            .as_str()
            .filter(|c| c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase()))
            .ok_or(INVALID)?;
        if let Some(i) = result.identity.as_mut() {
            for (key, label, kind) in [
                ("balanceCents", "Extra usage balance", "balance"),
                ("totalCents", "Extra usage total", "limit"),
                ("monthlyUsedCents", "Extra usage this month", "spend"),
            ] {
                let value = nonnegative(&wallet[key])? / 100.0;
                i.details
                    .push(detail(key, label, value, kind, Some(currency)));
            }
            let enabled = wallet["monthlyChargeLimitEnabled"]
                .as_bool()
                .ok_or(INVALID)?;
            i.details.push(detail(
                "monthlyChargeLimitEnabled",
                "Extra usage monthly charge limit",
                if enabled { "Enabled" } else { "Disabled" },
                "status",
                None,
            ));
            if enabled {
                i.details.push(detail(
                    "monthlyChargeLimitCents",
                    "Extra usage monthly charge limit",
                    nonnegative(&wallet["monthlyChargeLimitCents"])? / 100.0,
                    "limit",
                    Some(currency),
                ));
            }
        }
    }
    if result.windows.is_empty()
        && result
            .identity
            .as_ref()
            .map_or(true, |i| i.details.is_empty())
    {
        return Err(INVALID.into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn poe_is_an_aggregate_balance_not_a_quota() {
        let r = parse_poe(&json!({"current_point_balance":0})).unwrap();
        assert!(r.windows.is_empty());
        assert_eq!(r.identity.unwrap().details.len(), 1);
        for v in [
            json!({}),
            json!({"current_point_balance":null}),
            json!({"current_point_balance":-1}),
            json!({"current_point_balance":"42"}),
            json!({"current_point_balance":0,"error":{"message":"private"}}),
        ] {
            assert!(parse_poe(&v).is_err());
        }
    }

    #[test]
    fn moonshot_requires_success_and_preserves_negative_cash() {
        let mut v = json!({"code":0,"status":true,"data":{"available_balance":2.5,"voucher_balance":3.0,"cash_balance":-0.5}});
        let r = parse_moonshot(&v, "CNY").unwrap();
        assert!(r.windows.is_empty());
        assert_eq!(r.identity.unwrap().details.len(), 3);
        v["status"] = json!(false);
        assert!(parse_moonshot(&v, "USD").is_err());
        v["status"] = json!(true);
        v["data"]["voucher_balance"] = Value::Null;
        assert!(parse_moonshot(&v, "USD").is_err());
    }

    fn chutes_fixture() -> Value {
        json!({"subscription":true,"custom":false,"monthly_price":10,"four_hour":{"usage":2,"cap":8,"remaining":6,"reset_at":"2026-10-08T00:00:00Z"},"monthly":{"usage":15,"cap":20,"remaining":5,"reset_at":"2026-11-01T00:00:00Z"}})
    }
    #[test]
    fn chutes_uses_real_caps_and_handles_uncapped_and_unsubscribed() {
        let mut v = chutes_fixture();
        let r = parse_chutes(&v).unwrap();
        assert_eq!(r.windows[0].used_percent, 25.0);
        assert_eq!(
            r.windows[1].amount.as_ref().unwrap().unit.as_deref(),
            Some("USD")
        );
        v["custom"] = json!(true);
        v["monthly"] = json!({"uncapped":true});
        assert!(parse_chutes(&v).unwrap().windows[1].unlimited);
        assert!(parse_chutes(&json!({"subscription":false}))
            .unwrap()
            .windows
            .is_empty());
        for bad in [Value::Null, json!(0), json!(-1), json!("8")] {
            let mut v = chutes_fixture();
            v["four_hour"]["cap"] = bad;
            assert!(parse_chutes(&v).is_err());
        }
        assert!(parse_chutes(&json!({})).is_err());
    }

    #[test]
    fn zai_never_turns_unknown_windows_or_missing_percent_into_zero() {
        let r = parse_zai(&json!({"success":true,"code":200,"data":{"limits":[{"type":"TOKENS_LIMIT","percentage":36.5}]}})).unwrap();
        assert_eq!(r.windows[0].used_percent, 36.5);
        assert!(r.windows[0].resets_at.is_none());
        for entry in [
            json!({"type":"UNKNOWN_LIMIT","percentage":2}),
            json!({"type":"TOKENS_LIMIT"}),
            json!({"type":"TOKENS_LIMIT","percentage":101}),
            json!({"type":"TOKENS_LIMIT","percentage":null}),
        ] {
            assert!(parse_zai(&json!({"data":{"limits":[entry]}})).is_err());
        }
        assert!(parse_zai(
            &json!({"success":false,"data":{"limits":[{"type":"TOKENS_LIMIT","percentage":0}]}})
        )
        .is_err());
    }

    #[test]
    fn minimax_remaining_is_inverted_and_legacy_counts_are_not_guessed() {
        let mut v = json!({"base_resp":{"status_code":0},"model_remains":[{"model_name":"general","current_interval_remaining_percent":75,"end_time":1791417600000_i64,"current_weekly_remaining_percent":10}]});
        let r = parse_minimax(&v).unwrap();
        assert_eq!(r.windows[0].used_percent, 25.0);
        assert_eq!(r.windows[1].used_percent, 90.0);
        assert!(r.windows[0].resets_at.is_some());
        v["model_remains"][0]["current_weekly_status"] = json!(3);
        assert!(parse_minimax(&v).unwrap().windows[1].unlimited);
        v["base_resp"]["status_code"] = json!(1004);
        assert!(parse_minimax(&v).is_err());
        assert!(parse_minimax(&json!({"base_resp":{"status_code":0},"model_remains":[{"model_name":"general","current_interval_total_count":1500,"current_interval_usage_count":200}]})).is_err());
        assert!(
            parse_minimax(&json!({"base_resp":{"status_code":0},"model_remains":null})).is_err()
        );
    }

    #[test]
    fn minimax_not_in_plan_and_boost_are_not_false_full_quotas() {
        let r = parse_minimax(&json!({"base_resp":{"status_code":0},"model_remains":[{"model_name":"video","current_interval_status":3,"current_weekly_status":3,"current_interval_total_count":0,"current_weekly_total_count":0}]})).unwrap();
        assert!(r.windows.is_empty());
        assert_eq!(r.identity.unwrap().details.len(), 1);
        let r = parse_minimax(&json!({"base_resp":{"status_code":0},"model_remains":[{"model_name":"general","current_weekly_remaining_percent":90,"weekly_boost_permille":1500}]})).unwrap();
        assert!(r.windows.is_empty());
        assert_eq!(r.identity.unwrap().details.len(), 1);
    }

    #[test]
    fn kimi_ratios_and_cents_have_distinct_units() {
        let v = json!({"code":0,"data":{"kind":"ok","quota":{"usages":{"limit5h":{"usedRatio":0.125,"resetAt":"2026-10-08T00:00:00Z"}},"extraUsage":{"balanceCents":125,"totalCents":500,"monthlyUsedCents":375,"currency":"USD","monthlyChargeLimitEnabled":false}}}});
        let r = parse_kimi(&v).unwrap();
        assert_eq!(r.windows[0].used_percent, 12.5);
        assert_eq!(r.identity.unwrap().details.len(), 4);
        let mut invalid = v.clone();
        invalid["data"]["quota"]["usages"]["limit5h"]["usedRatio"] = Value::Null;
        assert!(parse_kimi(&invalid).is_err());
        invalid = v.clone();
        invalid["data"]["quota"]["usages"]["limit5h"]["usedRatio"] = json!(1.1);
        assert!(parse_kimi(&invalid).is_err());
        invalid = v;
        invalid["data"]["kind"] = json!("error");
        assert!(parse_kimi(&invalid).is_err());
        assert!(parse_kimi(
            &json!({"code":0,"data":{"kind":"ok","quota":{"usages":{},"extraUsage":null}}})
        )
        .is_err());
    }

    #[test]
    fn port_and_timestamp_parsing_cannot_redirect_or_invent_resets() {
        assert_eq!(local_port(None).unwrap(), 58627);
        assert_eq!(local_port(Some("12345")).unwrap(), 12345);
        for value in ["0", "65536", "localhost:80", "80/path", "-1"] {
            assert!(local_port(Some(value)).is_err());
        }
        assert!(date(&json!("tomorrow")).is_err());
        assert!(millis_date(&json!("1791417600000")).is_err());
        assert!(date(&Value::Null).unwrap().is_none());
    }

    #[test]
    fn balance_units_and_unspecified_windows_remain_explicit() {
        let moonshot = parse_moonshot(&json!({"code":0,"status":true,"data":{"available_balance":1.25,"cash_balance":1.0,"voucher_balance":0.25}}), "CNY").unwrap();
        let details = moonshot.identity.unwrap().details;
        assert_eq!(details[0].value, "1.25");
        assert_eq!(details[0].unit.as_deref(), Some("CNY"));
        let kimi = parse_kimi(&json!({"code":0,"data":{"kind":"ok","quota":{"usages":{},"extraUsage":{"balanceCents":125,"totalCents":500,"monthlyUsedCents":375,"currency":"USD","monthlyChargeLimitEnabled":false}}}})).unwrap();
        let details = kimi.identity.unwrap().details;
        assert_eq!(details[0].value, "1.25");
        assert_eq!(details[0].unit.as_deref(), Some("USD"));
        let credit =
            parse_zai(&json!({"data":{"limits":[{"type":"CREDIT_LIMIT","percentage":2}]}}))
                .unwrap();
        assert!(credit.windows.is_empty());
        assert_eq!(credit.identity.unwrap().details[0].value, "2");
    }

    #[test]
    fn malformed_percentages_and_provider_messages_cannot_leak() {
        for value in [json!(-1), json!(101), json!("50"), Value::Null] {
            let v = json!({"base_resp":{"status_code":0},"model_remains":[{"model_name":"general","current_interval_remaining_percent":value}]});
            assert!(parse_minimax(&v).is_err());
        }
        let error = parse_poe(&json!({"error":{"message":"private-account-secret"}})).unwrap_err();
        assert!(!error.contains("private-account-secret"));
    }

    #[test]
    fn minimax_key_type_selects_only_its_documented_read_endpoint() {
        assert_eq!(
            minimax_endpoint("sk-api-fixture"),
            ("https://api.minimax.io/account/query_balance", true)
        );
        assert_eq!(
            minimax_endpoint("sk-cp-fixture"),
            ("https://www.minimax.io/v1/token_plan/remains", false)
        );
        assert!(!minimax_endpoint("unknown-fixture").1);
    }

    #[test]
    fn minimax_payg_preserves_decimal_strings_without_inventing_currency() {
        let mut v = json!({"base_resp":{"status_code":0},"available_amount":"12.3400","cash_balance":"-0.25","voucher_balance":"1.00","credit_balance":"11.59","owed_amount":"0.25","balance_alert_switch":true,"balance_alert_threshold":"5.00"});
        let r = parse_minimax_balance(&v).unwrap();
        assert!(r.windows.is_empty());
        let details = r.identity.unwrap().details;
        assert_eq!(details[0].value, "12.3400");
        assert!(details.iter().all(|d| d.unit.is_none()));
        for bad in [
            Value::Null,
            json!(12.34),
            json!("NaN"),
            json!("inf"),
            json!("secret text"),
        ] {
            v["available_amount"] = bad;
            assert!(parse_minimax_balance(&v).is_err());
        }
        v["available_amount"] = json!("12.34");
        v["base_resp"]["status_code"] = json!(2062);
        assert!(parse_minimax_balance(&v).is_err());
    }
}
