use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchFieldSchema {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub tokenizer: String,
    pub stored: bool,
    pub indexed: bool,
    pub fast: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchIndexMeta {
    #[serde(default)]
    pub group: Option<String>,
    pub id: String,
    pub name: String,
    pub project_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub tokenizer: String,
    pub doc_count: usize,
    pub segments: usize,
    pub disk_bytes: u64,
    pub writer_budget_mb: usize,
    #[serde(default)]
    pub max_docs: Option<usize>,
    #[serde(default)]
    pub rate_limit_rpm: Option<u32>,
    pub created_at: i64,
    pub updated_at: i64,
    pub schema: Vec<SearchFieldSchema>,
}

/// Options for creating or updating a search index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchIndexOpts {
    pub group: Option<String>,
    pub project_id: Option<String>,
    pub description: Option<String>,
    pub tokenizer: Option<String>,
    pub writer_budget_mb: Option<usize>,
    pub max_docs: Option<usize>,
    pub rate_limit_rpm: Option<u32>,
    pub schema: Option<Vec<SearchFieldSchema>>,
    pub facet_fields: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchQueryInput {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub fuzzy: bool,
    pub sort: Option<String>,
    pub limit: Option<usize>,
    #[serde(default)]
    pub facets: HashMap<String, String>,
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    #[serde(default)]
    pub alpha: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub doc: serde_json::Value,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQueryResult {
    pub hits: Vec<SearchHit>,
    pub total: usize,
    pub took_ms: f64,
    pub facets: HashMap<String, HashMap<String, usize>>,
    pub tokenizer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbFtsIndexMeta {
    pub db_id: String,
    pub name: String,
    pub table: String,
    pub columns: Vec<String>,
    pub tokenizer: String,
    pub rows_indexed: usize,
    pub size_bytes: u64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbFtsHit {
    pub row: serde_json::Value,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbFtsQueryResult {
    pub index: DbFtsIndexMeta,
    pub rows: Vec<DbFtsHit>,
    pub sql: String,
    pub took_ms: f64,
}

pub fn default_ai_timeout_secs() -> u64 {
    120
}

/// Public metadata for a configured AI Gateway upstream provider (never exposes API keys).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProvider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub has_api_key: bool,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub fallback_provider: Option<String>,
    #[serde(default)]
    pub rate_limit_rpm: Option<u32>,
    #[serde(default)]
    pub cache_ttl_secs: u64,
    #[serde(default)]
    pub monthly_token_budget: Option<u64>,
    #[serde(default = "default_ai_timeout_secs")]
    pub timeout_secs: u64,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Internal persisted AI provider record containing optional AES-GCM encrypted API key.
#[derive(Debug, Clone)]
pub struct AiProviderRecord {
    pub provider: AiProvider,
    pub api_key_nonce: Option<Vec<u8>>,
    pub api_key_ciphertext: Option<Vec<u8>>,
}

/// Options for creating or updating an AI Gateway provider.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiProviderOpts {
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
    pub fallback_provider: Option<String>,
    pub rate_limit_rpm: Option<u32>,
    pub cache_ttl_secs: Option<u64>,
    pub monthly_token_budget: Option<u64>,
    pub timeout_secs: Option<u64>,
    pub enabled: Option<bool>,
}

/// Usage audit record emitted by the AI Gateway proxy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiUsageRecord {
    pub id: i64,
    pub provider_name: String,
    pub project_id: Option<String>,
    pub model: Option<String>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cached: bool,
    pub status: u16,
    pub latency_ms: u64,
    pub created_at: i64,
}

/// Aggregated AI Gateway token usage summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiUsageSummary {
    pub total_requests: usize,
    pub cached_requests: usize,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub month_tokens: u64,
    pub recent: Vec<AiUsageRecord>,
}

/// Configured outgoing alert webhook destination (never exposes HMAC secret).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertWebhook {
    pub id: String,
    pub name: String,
    pub url: String,
    pub has_secret: bool,
    #[serde(default)]
    pub events: Vec<String>,
    pub enabled: bool,
    #[serde(default)]
    pub last_status: Option<u16>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_triggered_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Internal persisted alert webhook record containing optional AES-GCM encrypted signing secret.
#[derive(Debug, Clone)]
pub struct AlertWebhookRecord {
    pub webhook: AlertWebhook,
    pub secret_nonce: Option<Vec<u8>>,
    pub secret_ciphertext: Option<Vec<u8>>,
}

/// Options for creating or updating an alert webhook.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AlertWebhookOpts {
    pub name: Option<String>,
    pub url: Option<String>,
    pub secret: Option<String>,
    pub clear_secret: Option<bool>,
    pub events: Option<Vec<String>>,
    pub enabled: Option<bool>,
}

fn default_backup_interval_hours() -> u32 {
    24
}

fn default_backup_max_retain() -> usize {
    7
}

/// Persisted scheduled automatic backup configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AutoBackupConfig {
    pub enabled: bool,
    #[serde(default = "default_backup_interval_hours")]
    pub interval_hours: u32,
    #[serde(default = "default_backup_max_retain")]
    pub max_backups: usize,
    pub last_backup_at_ms: Option<i64>,
}

impl Default for AutoBackupConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_hours: default_backup_interval_hours(),
            max_backups: default_backup_max_retain(),
            last_backup_at_ms: None,
        }
    }
}

/// Real-time Pub-Sub event delivered over `/_realtime/{channel}/sse`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealtimeMessage {
    pub id: String,
    pub channel: String,
    pub event: String,
    pub data: serde_json::Value,
    pub ts: i64,
}

/// Summary metadata for an active Realtime Pub-Sub channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealtimeChannelInfo {
    pub channel: String,
    pub subscribers: usize,
    pub published_total: u64,
    pub last_published_at_ms: Option<i64>,
}
