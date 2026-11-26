use std::time::{Duration, Instant};

use bytes::Bytes;
use chrono::{Datelike, TimeZone, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, OwnedSemaphorePermit};
use uuid::Uuid;

use crate::core::model::{
    default_ai_timeout_secs, AiProvider, AiProviderOpts, AiProviderRecord, AiUsageRecord,
    AiUsageSummary, ByteStream, Caller, FerroError, Result, TokenScope,
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

pub struct AiProxyStreamResponse {
    pub status: u16,
    pub content_type: String,
    pub provider_name: String,
    pub body: ByteStream,
}

pub enum AiProxyResult {
    Json(AiProxyResponse),
    Stream(AiProxyStreamResponse),
}

fn extract_sse_usage(
    line: &str,
    prompt_tokens: &mut u64,
    completion_tokens: &mut u64,
    total_tokens: &mut u64,
    model_seen: &mut Option<String>,
) {
    let trimmed = line.trim();
    let Some(data) = trimmed.strip_prefix("data:") else {
        return;
    };
    let payload = data.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return;
    }
    if let Ok(val) = serde_json::from_str::<Value>(payload) {
        if model_seen.is_none() {
            if let Some(m) = val.get("model").and_then(|v| v.as_str()) {
                if !m.trim().is_empty() {
                    *model_seen = Some(m.to_string());
                }
            }
        }
        if let Some(usage) = val.get("usage").filter(|u| u.is_object()) {
            let p = usage
                .get("prompt_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let c = usage
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let t = usage
                .get("total_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(p + c);
            if t > 0 || p > 0 || c > 0 {
                *prompt_tokens = p;
                *completion_tokens = c;
                *total_tokens = t;
            }
        }
    }
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
                timeout_secs: opts
                    .timeout_secs
                    .unwrap_or_else(default_ai_timeout_secs)
                    .clamp(5, 600),
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
        if let Some(ts) = opts.timeout_secs {
            rec.provider.timeout_secs = ts.clamp(5, 600);
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
        let project_filter = caller.project_id();
        let clean_provider = provider_name.map(str::trim).filter(|s| !s.is_empty());
        let recent = self
            .registry
            .list_ai_usage(clean_provider, project_filter, limit)
            .await?;
        let (
            total_requests,
            cached_requests,
            total_prompt_tokens,
            total_completion_tokens,
            total_tokens,
            month_tokens,
        ) = self
            .registry
            .aggregate_ai_usage(clean_provider, project_filter, month_start_ms())
            .await?;

        Ok(AiUsageSummary {
            total_requests,
            cached_requests,
            total_prompt_tokens,
            total_completion_tokens,
            total_tokens,
            month_tokens,
            recent,
        })
    }

    pub async fn list_ai_models(&self, caller: &Caller) -> Result<Value> {
        if matches!(caller.scope, TokenScope::Readonly) {
            return Err(FerroError::Forbidden(
                "readonly scope cannot query AI Gateway models".into(),
            ));
        }
        let providers = self.registry.list_ai_providers().await?;
        let mut data = Vec::new();
        for rec in providers {
            if !rec.provider.enabled {
                continue;
            }
            let model_id = rec
                .provider
                .default_model
                .clone()
                .unwrap_or_else(|| format!("{}/default", rec.provider.name));
            data.push(serde_json::json!({
                "id": model_id,
                "object": "model",
                "created": rec.provider.created_at / 1000,
                "owned_by": rec.provider.name,
                "ferropod_provider": rec.provider.name,
            }));
        }
        Ok(serde_json::json!({
            "object": "list",
            "data": data,
        }))
    }

    pub async fn clear_ai_cache(&self, caller: &Caller) -> Result<usize> {
        Self::require_admin(caller)?;
        let mut guard = self.ai_cache.write().await;
        let count = guard.len();
        guard.clear();
        drop(guard);
        self.audit(
            None,
            &caller.actor,
            "ai.cache.clear",
            serde_json::json!({ "evicted": count }),
        );
        Ok(count)
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

    fn build_provider_request(
        &self,
        rec: &AiProviderRecord,
        subpath: &str,
        body: &Value,
    ) -> reqwest::RequestBuilder {
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
        let timeout_secs = rec.provider.timeout_secs.clamp(5, 600);
        let mut req = self
            .ai_http_client
            .post(&target_url)
            .timeout(Duration::from_secs(timeout_secs))
            .json(&payload);
        if let (Some(nonce), Some(ct)) = (&rec.api_key_nonce, &rec.api_key_ciphertext) {
            if let Ok(key_bytes) = self.crypto.decrypt_secret(nonce, ct) {
                if let Ok(key_str) = String::from_utf8(key_bytes) {
                    req = req.bearer_auth(key_str);
                }
            }
        }
        req
    }

    async fn invoke_single_ai_provider(
        &self,
        rec: &AiProviderRecord,
        subpath: &str,
        body: &Value,
    ) -> std::result::Result<(u16, Value), String> {
        let resp = self
            .build_provider_request(rec, subpath, body)
            .send()
            .await
            .map_err(|e| format!("upstream connect failed: {e}"))?;
        let status = resp.status().as_u16();
        let val: Value = resp.json().await.unwrap_or_else(
            |e| serde_json::json!({ "error": format!("invalid upstream json: {e}") }),
        );
        Ok((status, val))
    }

    async fn send_streaming_ai_provider(
        &self,
        rec: &AiProviderRecord,
        subpath: &str,
        body: &Value,
    ) -> std::result::Result<reqwest::Response, String> {
        self.build_provider_request(rec, subpath, body)
            .send()
            .await
            .map_err(|e| format!("upstream connect failed: {e}"))
    }

    async fn enforce_ai_limits(&self, primary: &AiProviderRecord) -> Result<()> {
        self.enforce_service_rate_limit(
            &format!("ai:{}", primary.provider.name),
            primary.provider.rate_limit_rpm,
            super::DEFAULT_AI_RATE_LIMIT_RPM,
            &format!("AI provider '{}'", primary.provider.name),
        )?;

        if let Some(budget) = primary.provider.monthly_token_budget {
            let used = self
                .registry
                .sum_ai_tokens_since(&primary.provider.name, month_start_ms())
                .await?;
            if used >= budget {
                self.fire_alert(
                    "ai_budget_exceeded",
                    format!(
                        "AI provider '{}' monthly token budget exhausted",
                        primary.provider.name
                    ),
                    serde_json::json!({
                        "provider": primary.provider.name,
                        "used_tokens": used,
                        "monthly_token_budget": budget,
                    }),
                );
                return Err(FerroError::RateLimited(format!(
                    "AI provider '{}' monthly token budget ({budget}) exhausted",
                    primary.provider.name
                )));
            }
        }
        Ok(())
    }

    pub async fn ai_proxy(
        &self,
        caller: &Caller,
        requested_provider: Option<&str>,
        subpath: &str,
        mut body: Value,
    ) -> Result<AiProxyResponse> {
        if let Some(obj) = body.as_object_mut() {
            obj.remove("stream");
        }
        match self
            .ai_proxy_or_stream(caller, requested_provider, subpath, body)
            .await?
        {
            AiProxyResult::Json(resp) => Ok(resp),
            AiProxyResult::Stream(_) => Err(FerroError::InvalidArgument(
                "streaming not supported by this caller".into(),
            )),
        }
    }

    pub async fn ai_proxy_or_stream(
        &self,
        caller: &Caller,
        requested_provider: Option<&str>,
        subpath: &str,
        body: Value,
    ) -> Result<AiProxyResult> {
        if matches!(caller.scope, TokenScope::Readonly) {
            return Err(FerroError::Forbidden(
                "readonly scope cannot invoke AI Gateway".into(),
            ));
        }
        let primary = self.resolve_provider_record(requested_provider).await?;
        self.enforce_ai_limits(&primary).await?;
        let _ai_permit = self
            .acquire_timed_permit(
                self.ai_concurrency.clone(),
                format!(
                    "AI gateway is busy (max concurrency {} reached)",
                    super::DEFAULT_AI_MAX_CONCURRENCY
                ),
            )
            .await?;

        let is_stream = body
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_stream {
            return self
                .ai_proxy_stream(caller, primary, subpath, body, _ai_permit)
                .await
                .map(AiProxyResult::Stream);
        }

        let cache_enabled = primary.provider.cache_ttl_secs > 0;
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
                    return Ok(AiProxyResult::Json(AiProxyResponse {
                        status: 200,
                        body: cached_val.clone(),
                        provider_name: primary.provider.name,
                        cached: true,
                    }));
                }
            }
        }

        let started = Instant::now();
        let mut active = primary.clone();
        let (status, resp_val) = match self
            .invoke_single_ai_provider(&primary, subpath, &body)
            .await
        {
            Ok((st, val)) if st < 500 => (st, val),
            primary_fail => {
                if let Some(ref fb_name) = primary.provider.fallback_provider {
                    if let Ok(Some(fb_rec)) = self.registry.get_ai_provider(fb_name).await {
                        if fb_rec.provider.enabled {
                            match self
                                .invoke_single_ai_provider(&fb_rec, subpath, &body)
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

        Ok(AiProxyResult::Json(AiProxyResponse {
            status,
            body: resp_val,
            provider_name: active.provider.name,
            cached: false,
        }))
    }

    async fn ai_proxy_stream(
        &self,
        caller: &Caller,
        primary: AiProviderRecord,
        subpath: &str,
        body: Value,
        _ai_permit: OwnedSemaphorePermit,
    ) -> Result<AiProxyStreamResponse> {
        let started = Instant::now();
        let mut active = primary.clone();

        let upstream = match self
            .send_streaming_ai_provider(&primary, subpath, &body)
            .await
        {
            Ok(resp) if resp.status().as_u16() < 500 => resp,
            primary_fail => {
                if let Some(ref fb_name) = primary.provider.fallback_provider {
                    if let Ok(Some(fb_rec)) = self.registry.get_ai_provider(fb_name).await {
                        if fb_rec.provider.enabled {
                            match self
                                .send_streaming_ai_provider(&fb_rec, subpath, &body)
                                .await
                            {
                                Ok(resp2) => {
                                    active = fb_rec;
                                    resp2
                                }
                                Err(e2) => {
                                    return Err(FerroError::Internal(anyhow::anyhow!(
                                        "primary and fallback AI providers failed: {e2}"
                                    )));
                                }
                            }
                        } else {
                            match primary_fail {
                                Ok(resp) => resp,
                                Err(msg) => return Err(FerroError::Internal(anyhow::anyhow!(msg))),
                            }
                        }
                    } else {
                        match primary_fail {
                            Ok(resp) => resp,
                            Err(msg) => return Err(FerroError::Internal(anyhow::anyhow!(msg))),
                        }
                    }
                } else {
                    match primary_fail {
                        Ok(resp) => resp,
                        Err(msg) => return Err(FerroError::Internal(anyhow::anyhow!(msg))),
                    }
                }
            }
        };

        let status = upstream.status().as_u16();
        let content_type = upstream
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("text/event-stream; charset=utf-8")
            .to_string();

        let provider_name = active.provider.name.clone();
        let fallback_model = body
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .or_else(|| active.provider.default_model.clone());
        let project_id = caller.project_id().map(str::to_string);
        let svc = self.clone();

        let (tx, rx) = mpsc::channel::<std::io::Result<Bytes>>(32);
        let mut resp = upstream;
        let provider_for_task = provider_name.clone();
        tokio::spawn(async move {
            let _permit = _ai_permit;
            let mut sse_buf = String::new();
            let mut prompt_tokens = 0u64;
            let mut completion_tokens = 0u64;
            let mut total_tokens = 0u64;
            let mut model_seen: Option<String> = None;

            loop {
                match resp.chunk().await {
                    Ok(Some(chunk)) => {
                        if let Ok(text) = std::str::from_utf8(&chunk) {
                            sse_buf.push_str(text);
                            while let Some(pos) = sse_buf.find('\n') {
                                let line: String = sse_buf.drain(..=pos).collect();
                                extract_sse_usage(
                                    &line,
                                    &mut prompt_tokens,
                                    &mut completion_tokens,
                                    &mut total_tokens,
                                    &mut model_seen,
                                );
                            }
                        }
                        if tx.send(Ok(chunk)).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => {
                        if !sse_buf.is_empty() {
                            extract_sse_usage(
                                &sse_buf,
                                &mut prompt_tokens,
                                &mut completion_tokens,
                                &mut total_tokens,
                                &mut model_seen,
                            );
                        }
                        break;
                    }
                    Err(err) => {
                        let _ = tx.send(Err(std::io::Error::other(err.to_string()))).await;
                        break;
                    }
                }
            }

            let latency_ms = started.elapsed().as_millis() as u64;
            let _ = svc
                .registry
                .record_ai_usage(&AiUsageRecord {
                    id: 0,
                    provider_name: provider_for_task,
                    project_id,
                    model: model_seen.or(fallback_model),
                    prompt_tokens,
                    completion_tokens,
                    total_tokens,
                    cached: false,
                    status,
                    latency_ms,
                    created_at: Utc::now().timestamp_millis(),
                })
                .await;
        });

        let stream = futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        });

        Ok(AiProxyStreamResponse {
            status,
            content_type,
            provider_name,
            body: Box::pin(stream),
        })
    }
}
