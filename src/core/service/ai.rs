use std::time::Instant;

use chrono::{Datelike, TimeZone, Utc};
use reqwest::Client;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::core::model::{
    AiProvider, AiProviderOpts, AiProviderRecord, AiUsageRecord, AiUsageSummary, Caller,
    FerroError, Result,
};

use super::PlatformService;

fn month_start_ms() -> i64 {
    let now = Utc::now();
    Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0)
}

fn validate_base_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(FerroError::InvalidArgument(
            "AI provider base_url must start with http:// or https://".into(),
        ));
    }
    let _: reqwest::Url = trimmed
        .parse()
        .map_err(|e| FerroError::InvalidArgument(format!("invalid base_url: {e}")))?;
    Ok(trimmed.to_string())
}

fn cache_key_for(provider: &str, subpath: &str, body: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider.as_bytes());
    hasher.update(b":");
    hasher.update(subpath.as_bytes());
    hasher.update(b":");
    hasher.update(body.to_string().as_bytes());
    hex::encode(hasher.finalize())
}

pub struct AiProxyResponse {
    pub status: u16,
    pub body: Value,
    pub provider_name: String,
    pub cached: bool,
}

impl PlatformService {
    pub async fn create_ai_provider(
        &self,
        caller: &Caller,
        name: &str,
        opts: AiProviderOpts,
    ) -> Result<AiProvider> {
        Self::require_admin(caller)?;
        let clean_name = name.trim().to_string();
        if clean_name.is_empty() {
            return Err(FerroError::InvalidArgument(
                "AI provider name cannot be empty".into(),
            ));
        }
        let base_url = validate_base_url(
            opts.base_url
                .as_deref()
                .ok_or_else(|| FerroError::InvalidArgument("base_url is required".into()))?,
        )?;

        let (nonce, ct) = match opts
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(key) => {
                let (n, c) = self.crypto.encrypt_secret(key.as_bytes())?;
                (Some(n), Some(c))
            }
            None => (None, None),
        };
        let now = Utc::now().timestamp_millis();
        let rec = AiProviderRecord {
            provider: AiProvider {
                id: format!("aip_{}", &Uuid::new_v4().simple().to_string()[..12]),
                name: clean_name.clone(),
                base_url,
                has_api_key: ct.is_some(),
                default_model: opts
                    .default_model
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                fallback_provider: opts
                    .fallback_provider
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && *s != clean_name)
                    .map(str::to_owned),
                rate_limit_rpm: opts.rate_limit_rpm.filter(|v| *v > 0),
                cache_ttl_secs: opts.cache_ttl_secs.unwrap_or(0),
                monthly_token_budget: opts.monthly_token_budget.filter(|v| *v > 0),
                enabled: opts.enabled.unwrap_or(true),
                created_at: now,
                updated_at: now,
            },
            api_key_nonce: nonce,
            api_key_ciphertext: ct,
        };
        let saved = self.registry.upsert_ai_provider(rec).await?;
        self.audit(
            None,
            &caller.actor,
            "ai.provider.upsert",
            serde_json::json!({ "name": saved.provider.name }),
        );
        Ok(saved.provider)
    }

    pub async fn update_ai_provider(
        &self,
        caller: &Caller,
        name: &str,
        opts: AiProviderOpts,
    ) -> Result<AiProvider> {
        Self::require_admin(caller)?;
        let mut rec = self
            .registry
            .get_ai_provider(name)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("ai provider '{name}'")))?;

        if let Some(url) = opts.base_url.as_deref() {
            rec.provider.base_url = validate_base_url(url)?;
        }
        if let Some(key) = opts
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let (n, c) = self.crypto.encrypt_secret(key.as_bytes())?;
            rec.api_key_nonce = Some(n);
            rec.api_key_ciphertext = Some(c);
            rec.provider.has_api_key = true;
        }
        if let Some(model) = opts.default_model {
            let m = model.trim().to_string();
            rec.provider.default_model = if m.is_empty() { None } else { Some(m) };
        }
        if let Some(fb) = opts.fallback_provider {
            let f = fb.trim().to_string();
            rec.provider.fallback_provider = if f.is_empty() || f == rec.provider.name {
                None
            } else {
                Some(f)
            };
        }
        if let Some(rpm) = opts.rate_limit_rpm {
            rec.provider.rate_limit_rpm = if rpm == 0 { None } else { Some(rpm) };
        }
        if let Some(ttl) = opts.cache_ttl_secs {
            rec.provider.cache_ttl_secs = ttl;
        }
        if let Some(budget) = opts.monthly_token_budget {
            rec.provider.monthly_token_budget = if budget == 0 { None } else { Some(budget) };
        }
        if let Some(en) = opts.enabled {
            rec.provider.enabled = en;
        }
        rec.provider.updated_at = Utc::now().timestamp_millis();

        let saved = self.registry.upsert_ai_provider(rec).await?;
        self.audit(
            None,
            &caller.actor,
            "ai.provider.update",
            serde_json::json!({ "name": saved.provider.name }),
        );
        Ok(saved.provider)
    }

    pub async fn list_ai_providers(&self, caller: &Caller) -> Result<Vec<AiProvider>> {
        if !caller.scope.allows_read() {
            return Err(FerroError::Forbidden("insufficient scope".into()));
        }
        Ok(self
            .registry
            .list_ai_providers()
            .await?
            .into_iter()
            .map(|r| r.provider)
            .collect())
    }

    pub async fn delete_ai_provider(&self, caller: &Caller, name: &str) -> Result<()> {
        Self::require_admin(caller)?;
        let deleted = self.registry.delete_ai_provider(name).await?;
        if !deleted {
            return Err(FerroError::NotFound(format!("ai provider '{name}'")));
        }
        self.audit(
            None,
            &caller.actor,
            "ai.provider.delete",
            serde_json::json!({ "name": name }),
        );
        Ok(())
    }

    pub async fn list_ai_usage(
        &self,
        caller: &Caller,
        provider_name: Option<&str>,
        limit: usize,
    ) -> Result<AiUsageSummary> {
        if !caller.scope.allows_read() {
            return Err(FerroError::Forbidden("insufficient scope".into()));
        }
        let recent = self.registry.list_ai_usage(provider_name, limit).await?;
        let mut summary = AiUsageSummary {
            total_requests: recent.len(),
            ..Default::default()
        };
        for item in &recent {
            if item.cached {
                summary.cached_requests += 1;
            }
            summary.total_prompt_tokens += item.prompt_tokens;
            summary.total_completion_tokens += item.completion_tokens;
            summary.total_tokens += item.total_tokens;
        }
        summary.recent = recent;
        Ok(summary)
    }

    async fn resolve_provider_record(&self, requested: Option<&str>) -> Result<AiProviderRecord> {
        if let Some(name) = requested.map(str::trim).filter(|s| !s.is_empty()) {
            let rec = self
                .registry
                .get_ai_provider(name)
                .await?
                .ok_or_else(|| FerroError::NotFound(format!("ai provider '{name}'")))?;
            if !rec.provider.enabled {
                return Err(FerroError::InvalidArgument(format!(
                    "ai provider '{name}' is disabled"
                )));
            }
            return Ok(rec);
        }
        self.registry
            .list_ai_providers()
            .await?
            .into_iter()
            .find(|r| r.provider.enabled)
            .ok_or_else(|| FerroError::NotFound("no enabled AI provider configured".into()))
    }

    async fn invoke_single_ai_provider(
        &self,
        client: &Client,
        rec: &AiProviderRecord,
        subpath: &str,
        body: &Value,
    ) -> std::result::Result<(u16, Value), String> {
        let mut payload = body.clone();
        if let Some(obj) = payload.as_object_mut() {
            if !obj.contains_key("model") {
                if let Some(ref m) = rec.provider.default_model {
                    obj.insert("model".into(), Value::String(m.clone()));
                }
            }
        }
        let target_url = format!(
            "{}/{}",
            rec.provider.base_url.trim_end_matches('/'),
            subpath.trim_start_matches('/')
        );
        let mut req = client.post(&target_url).json(&payload);
        if let (Some(nonce), Some(ct)) = (&rec.api_key_nonce, &rec.api_key_ciphertext) {
            if let Ok(key_bytes) = self.crypto.decrypt_secret(nonce, ct) {
                if let Ok(key_str) = String::from_utf8(key_bytes) {
                    req = req.bearer_auth(key_str);
                }
            }
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("upstream connect failed: {e}"))?;
        let status = resp.status().as_u16();
        let val: Value = resp.json().await.unwrap_or_else(
            |e| serde_json::json!({ "error": format!("invalid upstream json: {e}") }),
        );
        Ok((status, val))
    }

    pub async fn ai_proxy(
        &self,
        caller: &Caller,
        requested_provider: Option<&str>,
        subpath: &str,
        body: Value,
    ) -> Result<AiProxyResponse> {
        if !caller.scope.allows_read() {
            return Err(FerroError::Forbidden("insufficient scope".into()));
        }
        let primary = self.resolve_provider_record(requested_provider).await?;

        // Enforce per-provider RPM limit
        if let Some(rpm) = primary.provider.rate_limit_rpm {
            let key = format!("ai:{}", primary.provider.name);
            let (allowed, _, reset) = self.rate_limiter.check_and_record(&key, rpm);
            if !allowed {
                return Err(FerroError::RateLimited(format!(
                    "AI provider '{}' RPM limit exceeded (retry in {reset}s)",
                    primary.provider.name
                )));
            }
        }

        // Enforce monthly token budget
        if let Some(budget) = primary.provider.monthly_token_budget {
            let used = self
                .registry
                .sum_ai_tokens_since(&primary.provider.name, month_start_ms())
                .await?;
            if used >= budget {
                return Err(FerroError::RateLimited(format!(
                    "AI provider '{}' monthly token budget ({budget}) exhausted",
                    primary.provider.name
                )));
            }
        }

        let is_stream = body
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let cache_enabled = primary.provider.cache_ttl_secs > 0 && !is_stream;
        let ckey = cache_key_for(&primary.provider.name, subpath, &body);
        let now_ms = Utc::now().timestamp_millis();

        if cache_enabled {
            let cache = self.ai_cache.read().await;
            if let Some((exp_ms, cached_val)) = cache.get(&ckey) {
                if now_ms < *exp_ms {
                    let model = cached_val
                        .get("model")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                        .or_else(|| primary.provider.default_model.clone());
                    let _ = self
                        .registry
                        .record_ai_usage(&AiUsageRecord {
                            id: 0,
                            provider_name: primary.provider.name.clone(),
                            project_id: caller.project_id().map(str::to_string),
                            model,
                            prompt_tokens: 0,
                            completion_tokens: 0,
                            total_tokens: 0,
                            cached: true,
                            status: 200,
                            latency_ms: 0,
                            created_at: now_ms,
                        })
                        .await;
                    return Ok(AiProxyResponse {
                        status: 200,
                        body: cached_val.clone(),
                        provider_name: primary.provider.name,
                        cached: true,
                    });
                }
            }
        }

        let started = Instant::now();
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| FerroError::Internal(anyhow::anyhow!("ai client init: {e}")))?;

        let mut active = primary.clone();
        let (status, resp_val) = match self
            .invoke_single_ai_provider(&client, &primary, subpath, &body)
            .await
        {
            Ok((st, val)) if st < 500 => (st, val),
            primary_fail => {
                if let Some(ref fb_name) = primary.provider.fallback_provider {
                    if let Ok(Some(fb_rec)) = self.registry.get_ai_provider(fb_name).await {
                        if fb_rec.provider.enabled {
                            match self
                                .invoke_single_ai_provider(&client, &fb_rec, subpath, &body)
                                .await
                            {
                                Ok((st2, val2)) => {
                                    active = fb_rec;
                                    (st2, val2)
                                }
                                Err(e2) => {
                                    return Err(FerroError::Internal(anyhow::anyhow!(
                                        "primary and fallback AI providers failed: {e2}"
                                    )));
                                }
                            }
                        } else {
                            match primary_fail {
                                Ok(pair) => pair,
                                Err(msg) => return Err(FerroError::Internal(anyhow::anyhow!(msg))),
                            }
                        }
                    } else {
                        match primary_fail {
                            Ok(pair) => pair,
                            Err(msg) => return Err(FerroError::Internal(anyhow::anyhow!(msg))),
                        }
                    }
                } else {
                    match primary_fail {
                        Ok(pair) => pair,
                        Err(msg) => return Err(FerroError::Internal(anyhow::anyhow!(msg))),
                    }
                }
            }
        };

        let latency_ms = started.elapsed().as_millis() as u64;
        let usage = resp_val.get("usage");
        let prompt_tokens = usage
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let completion_tokens = usage
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let total_tokens = usage
            .and_then(|u| u.get("total_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(prompt_tokens + completion_tokens);
        let model = resp_val
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .or_else(|| {
                body.get("model")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            })
            .or_else(|| active.provider.default_model.clone());

        let _ = self
            .registry
            .record_ai_usage(&AiUsageRecord {
                id: 0,
                provider_name: active.provider.name.clone(),
                project_id: caller.project_id().map(str::to_string),
                model,
                prompt_tokens,
                completion_tokens,
                total_tokens,
                cached: false,
                status,
                latency_ms,
                created_at: Utc::now().timestamp_millis(),
            })
            .await;

        if cache_enabled && status == 200 {
            let mut cache = self.ai_cache.write().await;
            if cache.len() >= 512 {
                let cutoff = Utc::now().timestamp_millis();
                cache.retain(|_, (exp, _)| *exp > cutoff);
                if cache.len() >= 512 {
                    if let Some(first_k) = cache.keys().next().cloned() {
                        cache.remove(&first_k);
                    }
                }
            }
            let exp =
                Utc::now().timestamp_millis() + (primary.provider.cache_ttl_secs as i64 * 1000);
            cache.insert(ckey, (exp, resp_val.clone()));
        }

        Ok(AiProxyResponse {
            status,
            body: resp_val,
            provider_name: active.provider.name,
            cached: false,
        })
    }
}
