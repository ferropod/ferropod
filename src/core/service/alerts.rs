use std::time::{Duration, Instant};

use chrono::Utc;
use tracing::warn;
use uuid::Uuid;

use crate::core::crypto::Crypto;
use crate::core::model::{
    AlertTestResult, AlertWebhook, AlertWebhookOpts, AlertWebhookRecord, Caller, FerroError, Result,
};

use super::PlatformService;

fn validate_webhook_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(FerroError::InvalidArgument(
            "webhook URL must start with http:// or https://".into(),
        ));
    }
    let _: reqwest::Url = trimmed
        .parse()
        .map_err(|e| FerroError::InvalidArgument(format!("invalid webhook URL: {e}")))?;
    Ok(trimmed.to_string())
}

fn normalize_webhook_events(events: Option<Vec<String>>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(items) = events {
        for raw in items {
            let trimmed = raw.trim();
            if !trimmed.is_empty() && !out.iter().any(|existing| existing == trimmed) {
                out.push(trimmed.to_string());
            }
        }
    }
    if out.is_empty() {
        out.push("*".to_string());
    }
    out
}

impl PlatformService {
    pub async fn list_alert_webhooks(&self, caller: &Caller) -> Result<Vec<AlertWebhook>> {
        Self::require_admin(caller)?;
        Ok(self
            .registry
            .list_alert_webhooks()
            .await?
            .into_iter()
            .map(|r| r.webhook)
            .collect())
    }

    pub async fn create_alert_webhook(
        &self,
        caller: &Caller,
        opts: AlertWebhookOpts,
    ) -> Result<AlertWebhook> {
        Self::require_admin(caller)?;
        let name = opts
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| FerroError::InvalidArgument("webhook name cannot be empty".into()))?
            .to_string();
        let url = validate_webhook_url(
            opts.url
                .as_deref()
                .ok_or_else(|| FerroError::InvalidArgument("webhook url is required".into()))?,
        )?;
        let events = normalize_webhook_events(opts.events);
        let (nonce, ct) = match opts
            .secret
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(sec) => {
                let (n, c) = self.crypto.encrypt_secret(sec.as_bytes())?;
                (Some(n), Some(c))
            }
            None => (None, None),
        };
        let now = Utc::now().timestamp_millis();
        let rec = AlertWebhookRecord {
            webhook: AlertWebhook {
                id: format!("whk_{}", &Uuid::new_v4().simple().to_string()[..12]),
                name,
                url,
                has_secret: ct.is_some(),
                events,
                enabled: opts.enabled.unwrap_or(true),
                last_status: None,
                last_error: None,
                last_triggered_at: None,
                created_at: now,
                updated_at: now,
            },
            secret_nonce: nonce,
            secret_ciphertext: ct,
        };
        let saved = self.registry.upsert_alert_webhook(rec).await?;
        self.audit(
            None,
            &caller.actor,
            "alerts.webhook.create",
            serde_json::json!({ "id": saved.webhook.id, "name": saved.webhook.name }),
        );
        Ok(saved.webhook)
    }

    pub async fn update_alert_webhook(
        &self,
        caller: &Caller,
        id: &str,
        opts: AlertWebhookOpts,
    ) -> Result<AlertWebhook> {
        Self::require_admin(caller)?;
        let mut rec = self
            .registry
            .get_alert_webhook(id)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("alert webhook '{id}'")))?;

        if let Some(name) = opts.name.as_deref().map(str::trim) {
            if name.is_empty() {
                return Err(FerroError::InvalidArgument(
                    "webhook name cannot be empty".into(),
                ));
            }
            rec.webhook.name = name.to_string();
        }
        if let Some(url) = opts.url.as_deref() {
            rec.webhook.url = validate_webhook_url(url)?;
        }
        if opts.clear_secret == Some(true) {
            rec.secret_nonce = None;
            rec.secret_ciphertext = None;
            rec.webhook.has_secret = false;
        } else if let Some(sec) = opts
            .secret
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let (n, c) = self.crypto.encrypt_secret(sec.as_bytes())?;
            rec.secret_nonce = Some(n);
            rec.secret_ciphertext = Some(c);
            rec.webhook.has_secret = true;
        }
        if let Some(events) = opts.events {
            rec.webhook.events = normalize_webhook_events(Some(events));
        }
        if let Some(enabled) = opts.enabled {
            rec.webhook.enabled = enabled;
        }
        rec.webhook.updated_at = Utc::now().timestamp_millis();

        let saved = self.registry.upsert_alert_webhook(rec).await?;
        self.audit(
            None,
            &caller.actor,
            "alerts.webhook.update",
            serde_json::json!({ "id": saved.webhook.id, "name": saved.webhook.name }),
        );
        Ok(saved.webhook)
    }

    pub async fn delete_alert_webhook(&self, caller: &Caller, id: &str) -> Result<()> {
        Self::require_admin(caller)?;
        let deleted = self.registry.delete_alert_webhook(id).await?;
        if !deleted {
            return Err(FerroError::NotFound(format!("alert webhook '{id}'")));
        }
        self.audit(
            None,
            &caller.actor,
            "alerts.webhook.delete",
            serde_json::json!({ "id": id }),
        );
        Ok(())
    }

    async fn deliver_to_webhook_record(
        &self,
        rec: &AlertWebhookRecord,
        event: &str,
        title: &str,
        detail: &serde_json::Value,
    ) -> AlertTestResult {
        let eff = self.effective_settings();
        let payload = serde_json::json!({
            "event": event,
            "instance": eff.instance_name.as_deref().unwrap_or("ferropod"),
            "title": title,
            "detail": detail,
            "timestamp": Utc::now().to_rfc3339(),
        });
        let body_bytes = serde_json::to_vec(&payload).unwrap_or_default();

        let secret = match (&rec.secret_nonce, &rec.secret_ciphertext) {
            (Some(n), Some(c)) => self
                .crypto
                .decrypt_secret(n, c)
                .ok()
                .and_then(|b| String::from_utf8(b).ok()),
            _ => None,
        };

        let started = Instant::now();
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                let err_str = format!("build HTTP client failed: {e}");
                let _ = self
                    .registry
                    .record_alert_webhook_delivery(
                        &rec.webhook.id,
                        None,
                        Some(&err_str),
                        Utc::now().timestamp_millis(),
                    )
                    .await;
                return AlertTestResult {
                    ok: false,
                    status_code: None,
                    latency_ms: started.elapsed().as_millis() as u64,
                    error: Some(err_str),
                };
            }
        };

        let mut req = client
            .post(&rec.webhook.url)
            .header("content-type", "application/json")
            .header("x-ferropod-event", event);
        if let Some(sec) = secret {
            let sig = Crypto::hmac_sha256_hex(sec.as_bytes(), &body_bytes);
            req = req.header("x-ferropod-signature", format!("sha256={sig}"));
        }

        let res = match req.body(body_bytes).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let ok = resp.status().is_success();
                AlertTestResult {
                    ok,
                    status_code: Some(status),
                    latency_ms: started.elapsed().as_millis() as u64,
                    error: if ok {
                        None
                    } else {
                        Some(format!("webhook endpoint returned HTTP {status}"))
                    },
                }
            }
            Err(e) => AlertTestResult {
                ok: false,
                status_code: None,
                latency_ms: started.elapsed().as_millis() as u64,
                error: Some(e.to_string()),
            },
        };

        let _ = self
            .registry
            .record_alert_webhook_delivery(
                &rec.webhook.id,
                res.status_code,
                res.error.as_deref(),
                Utc::now().timestamp_millis(),
            )
            .await;

        res
    }

    pub async fn test_alert_webhook_by_id(
        &self,
        caller: &Caller,
        id: &str,
    ) -> Result<AlertTestResult> {
        Self::require_admin(caller)?;
        let rec = self
            .registry
            .get_alert_webhook(id)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("alert webhook '{id}'")))?;
        let res = self
            .deliver_to_webhook_record(
                &rec,
                "test",
                "FerroPod test alert",
                &serde_json::json!({ "actor": caller.actor, "webhook": rec.webhook.name }),
            )
            .await;
        self.audit(
            None,
            &caller.actor,
            "alerts.webhook.test",
            serde_json::json!({ "id": id, "ok": res.ok, "status_code": res.status_code }),
        );
        Ok(res)
    }

    pub(super) async fn dispatch_alert_webhooks(
        &self,
        event: &str,
        title: &str,
        detail: &serde_json::Value,
    ) {
        let records = match self.registry.list_alert_webhooks().await {
            Ok(list) => list,
            Err(e) => {
                warn!(error = %e, "failed to load alert webhooks");
                return;
            }
        };
        for rec in records {
            if !rec.webhook.enabled {
                continue;
            }
            let matches = rec.webhook.events.is_empty()
                || rec.webhook.events.iter().any(|ev| ev == "*" || ev == event);
            if !matches {
                continue;
            }
            let res = self
                .deliver_to_webhook_record(&rec, event, title, detail)
                .await;
            if !res.ok {
                warn!(
                    webhook_id = %rec.webhook.id,
                    event = %event,
                    error = ?res.error,
                    status = ?res.status_code,
                    "registered alert webhook delivery failed"
                );
            }
        }
    }
}
