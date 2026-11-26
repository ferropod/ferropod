use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{FerroError, Result};

/// Logical binding declarations for a deployment (logical name -> physical resource name).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Bindings {
    #[serde(default)]
    pub kv: HashMap<String, String>,
    #[serde(default)]
    pub db: HashMap<String, String>,
    #[serde(default)]
    pub queue: HashMap<String, String>,
    #[serde(default)]
    pub bucket: HashMap<String, String>,
    #[serde(default)]
    pub search: HashMap<String, String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Project entity stored in `core.db`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    #[serde(default)]
    pub group: Option<String>,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub active_deployment_id: Option<String>,
    pub preview_deployment_id: Option<String>,
    pub max_concurrency: i64,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub wasm_max_memory_mb: Option<usize>,
    #[serde(default)]
    pub allow_private_outbound: bool,
    pub access_policy_json: Option<String>,
    pub health_check_path: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Options for creating or updating a project/app.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectOpts {
    pub group: Option<String>,
    pub description: Option<String>,
    pub health_check_path: Option<String>,
    pub max_concurrency: Option<i64>,
    pub timeout_ms: Option<u64>,
    pub wasm_max_memory_mb: Option<usize>,
    pub allow_private_outbound: Option<bool>,
}

/// Immutable deployment version record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub id: String,
    pub project_id: String,
    pub artifact_path: Option<String>,
    pub assets_bucket: Option<String>,
    pub assets_prefix: Option<String>,
    pub spa_fallback: bool,
    pub bindings: Bindings,
    pub crons: Vec<String>,
    pub status: String,
    pub created_at: i64,
}

/// Target environment for a routed hostname.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetEnv {
    #[default]
    Prod,
    Preview,
}

impl TargetEnv {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Prod => "prod",
            Self::Preview => "preview",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "prod" => Ok(Self::Prod),
            "preview" => Ok(Self::Preview),
            other => Err(FerroError::InvalidArgument(format!(
                "invalid target_env '{other}', expected 'prod' or 'preview'"
            ))),
        }
    }
}

/// Hostname-to-project route mapping with optional reverse proxy, rate limit, Basic Auth, and canary split.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Route {
    pub host: String,
    pub project_id: String,
    pub target_env: TargetEnv,
    #[serde(default)]
    pub proxy_prefix: Option<String>,
    #[serde(default)]
    pub proxy_upstream: Option<String>,
    #[serde(default)]
    pub rate_limit_rpm: Option<u32>,
    #[serde(default)]
    pub basic_auth_user: Option<String>,
    #[serde(default, skip_serializing)]
    pub basic_auth_pass_hash: Option<String>,
    #[serde(default)]
    pub basic_auth_enabled: bool,
    #[serde(default)]
    pub canary_deployment_id: Option<String>,
    #[serde(default)]
    pub canary_weight: u8,
}

/// Optional route policy overrides for creating/updating a hostname route.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RouteOpts {
    pub proxy_prefix: Option<String>,
    pub proxy_upstream: Option<String>,
    pub rate_limit_rpm: Option<u32>,
    pub basic_auth_user: Option<String>,
    pub basic_auth_password: Option<String>,
    pub clear_basic_auth: Option<bool>,
    pub canary_deployment_id: Option<String>,
    pub canary_weight: Option<u8>,
}

/// Cached entry in the hot in-memory routing table.
#[derive(Debug, Clone)]
pub struct RouteEntry {
    pub host: String,
    pub project_id: String,
    pub target_env: TargetEnv,
    pub active_deployment: Option<Deployment>,
    pub proxy_prefix: Option<String>,
    pub proxy_upstream: Option<String>,
    pub rate_limit_rpm: Option<u32>,
    pub basic_auth_user: Option<String>,
    pub basic_auth_pass_hash: Option<String>,
    pub canary_deployment: Option<Deployment>,
    pub canary_weight: u8,
}

/// API / MCP access token metadata (raw secret is never persisted).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiToken {
    pub id: String,
    pub name: String,
    pub scope: TokenScope,
    pub expires_at: Option<i64>,
    pub last_used_at: Option<i64>,
    pub created_at: i64,
}

/// Token authorization scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenScope {
    Admin,
    Project(String),
    Readonly,
}

impl TokenScope {
    pub fn as_str(&self) -> String {
        match self {
            Self::Admin => "admin".to_string(),
            Self::Project(id) => format!("project:{id}"),
            Self::Readonly => "readonly".to_string(),
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        if s == "admin" {
            Ok(Self::Admin)
        } else if s == "readonly" {
            Ok(Self::Readonly)
        } else if let Some(pid) = s.strip_prefix("project:") {
            if pid.is_empty() {
                return Err(FerroError::InvalidArgument("empty project scope".into()));
            }
            Ok(Self::Project(pid.to_string()))
        } else {
            Err(FerroError::InvalidArgument(format!(
                "invalid scope '{s}', expected 'admin', 'readonly', or 'project:<id>'"
            )))
        }
    }

    pub fn allows_admin(&self) -> bool {
        matches!(self, Self::Admin)
    }

    pub fn allows_project_read(&self, project_id: &str) -> bool {
        match self {
            Self::Admin | Self::Readonly => true,
            Self::Project(pid) => pid == project_id,
        }
    }

    pub fn allows_project_write(&self, project_id: &str) -> bool {
        match self {
            Self::Admin => true,
            Self::Project(pid) => pid == project_id,
            Self::Readonly => false,
        }
    }

    pub fn allows_resource_read(&self, owner_project_id: Option<&str>) -> bool {
        match self {
            Self::Admin | Self::Readonly => true,
            Self::Project(pid) => owner_project_id == Some(pid.as_str()),
        }
    }

    pub fn allows_resource_write(&self, owner_project_id: Option<&str>) -> bool {
        match self {
            Self::Admin => true,
            Self::Project(pid) => owner_project_id == Some(pid.as_str()),
            Self::Readonly => false,
        }
    }

    pub fn allows_read(&self) -> bool {
        true
    }
}

/// Authenticated caller identity passed into `PlatformService` by all adapters
/// (REST API, Blob API, CLI, future MCP server, and future Wasmtime host bindings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub actor: String,
    pub scope: TokenScope,
}

impl Caller {
    pub fn from_token(token: &ApiToken) -> Self {
        Self {
            actor: token.name.clone(),
            scope: token.scope.clone(),
        }
    }

    pub fn system(actor: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            scope: TokenScope::Admin,
        }
    }

    pub fn project(actor: impl Into<String>, project_id: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            scope: TokenScope::Project(project_id.into()),
        }
    }

    pub fn readonly(actor: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            scope: TokenScope::Readonly,
        }
    }

    pub fn project_id(&self) -> Option<&str> {
        match &self.scope {
            TokenScope::Project(pid) => Some(pid.as_str()),
            _ => None,
        }
    }
}

/// Authorization context for Blob operations (supports Bearer caller, HMAC presigned URL, or anonymous public read).
#[derive(Debug, Clone)]
pub enum BlobAuth<'a> {
    Caller(&'a Caller),
    Presigned { op: &'a str, exp: i64, sig: &'a str },
    Anonymous,
}
