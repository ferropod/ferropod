use chrono::Utc;

use crate::core::model::{
    Caller, FerroError, Result, SearchFieldSchema, SearchIndexMeta, SearchIndexOpts,
    SearchQueryInput, SearchQueryResult, TokenScope,
};
use crate::storage::search_store::TursoSearchStore;

use super::PlatformService;

impl PlatformService {
    async fn resolve_search_index(&self, id_or_name: &str) -> Result<SearchIndexMeta> {
        self.search
            .get_index(id_or_name)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("search index '{id_or_name}'")))
    }

    pub async fn list_search_indexes(&self, caller: &Caller) -> Result<Vec<SearchIndexMeta>> {
        let all = self.search.list_indexes().await?;
        match &caller.scope {
            TokenScope::Admin | TokenScope::Readonly => Ok(all),
            TokenScope::Project(pid) => Ok(all
                .into_iter()
                .filter(|idx| idx.project_id.as_deref() == Some(pid.as_str()))
                .collect()),
        }
    }

    pub async fn create_search_index(
        &self,
        caller: &Caller,
        name: &str,
        project_id: Option<&str>,
        tokenizer: Option<&str>,
        schema: Option<Vec<SearchFieldSchema>>,
    ) -> Result<SearchIndexMeta> {
        self.create_search_index_with_opts(
            caller,
            name,
            SearchIndexOpts {
                project_id: project_id.map(str::to_owned),
                tokenizer: tokenizer.map(str::to_owned),
                schema,
                ..Default::default()
            },
        )
        .await
    }

    pub async fn create_search_index_with_opts(
        &self,
        caller: &Caller,
        name: &str,
        mut opts: SearchIndexOpts,
    ) -> Result<SearchIndexMeta> {
        opts.group = super::normalize_resource_group(opts.group.as_deref())?.map(str::to_owned);
        let name = name.trim();
        if name.is_empty() {
            return Err(FerroError::InvalidArgument(
                "search index name cannot be empty".into(),
            ));
        }
        let resolved_pid = match (&caller.scope, opts.project_id.as_deref()) {
            (TokenScope::Admin, pid) => self.resolve_optional_project_id(pid).await?,
            (TokenScope::Project(_), Some(req_pid)) => {
                let proj = self.get_project_internal(req_pid).await?;
                Self::require_project_write(caller, &proj.id)?;
                Some(proj.id)
            }
            (TokenScope::Project(scope_pid), None) => Some(scope_pid.clone()),
            (TokenScope::Readonly, _) => {
                return Err(FerroError::Forbidden(
                    "readonly token cannot create search indexes".into(),
                ));
            }
        };
        opts.project_id = resolved_pid.clone();

        let meta = self.search.create_index_with_opts(name, opts).await?;
        self.audit(
            resolved_pid.as_deref(),
            &caller.actor,
            "search.index.create",
            serde_json::json!({ "id": meta.id, "name": meta.name, "tokenizer": meta.tokenizer }),
        );
        Ok(meta)
    }

    pub async fn update_search_index(
        &self,
        caller: &Caller,
        id_or_name: &str,
        opts: SearchIndexOpts,
    ) -> Result<SearchIndexMeta> {
        let mut meta = self.resolve_search_index(id_or_name).await?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("search index '{}'", meta.name),
        )?;
        if let Some(g) = opts.group {
            meta.group = super::normalize_resource_group(Some(&g))?.map(str::to_owned);
        }
        if let Some(desc) = opts.description {
            let trimmed = desc.trim();
            meta.description = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Some(tok) = opts.tokenizer {
            let trimmed = tok.trim();
            if !trimmed.is_empty() {
                meta.tokenizer = trimmed.to_string();
            }
        }
        if let Some(wb) = opts.writer_budget_mb {
            if wb > 0 {
                meta.writer_budget_mb = wb.clamp(1, 256);
            }
        }
        if let Some(schema) = opts.schema {
            if !schema.is_empty() {
                meta.schema = schema;
            }
        } else if let Some(facets) = opts.facet_fields {
            meta.schema =
                TursoSearchStore::build_schema_with_facets(&meta.tokenizer, Some(&facets));
        }
        meta.updated_at = Utc::now().timestamp_millis();
        self.search.update_index(&meta).await?;
        self.audit(
            meta.project_id.as_deref(),
            &caller.actor,
            "search.index.update",
            serde_json::json!({ "id": meta.id, "name": meta.name, "tokenizer": meta.tokenizer }),
        );
        Ok(meta)
    }

    pub async fn delete_search_index(&self, caller: &Caller, id_or_name: &str) -> Result<()> {
        let meta = self.resolve_search_index(id_or_name).await?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("search index '{}'", meta.name),
        )?;
        self.search.delete_index(&meta.id).await?;
        self.audit(
            meta.project_id.as_deref(),
            &caller.actor,
            "search.index.delete",
            serde_json::json!({ "id": meta.id, "name": meta.name }),
        );
        Ok(())
    }

    pub async fn commit_search_index(
        &self,
        caller: &Caller,
        id_or_name: &str,
    ) -> Result<SearchIndexMeta> {
        let meta = self.resolve_search_index(id_or_name).await?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("search index '{}'", meta.name),
        )?;
        let updated = self.search.commit_index(&meta.id).await?;
        self.audit(
            meta.project_id.as_deref(),
            &caller.actor,
            "search.index.commit",
            serde_json::json!({ "id": updated.id }),
        );
        Ok(updated)
    }

    pub async fn list_search_docs(
        &self,
        caller: &Caller,
        id_or_name: &str,
    ) -> Result<Vec<serde_json::Value>> {
        let meta = self.resolve_search_index(id_or_name).await?;
        Self::require_resource_read(
            caller,
            meta.project_id.as_deref(),
            &format!("search index '{}'", meta.name),
        )?;
        self.search.list_docs(&meta.id).await
    }

    pub async fn list_search_docs_page(
        &self,
        caller: &Caller,
        id_or_name: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<serde_json::Value>> {
        let meta = self.resolve_search_index(id_or_name).await?;
        Self::require_resource_read(
            caller,
            meta.project_id.as_deref(),
            &format!("search index '{}'", meta.name),
        )?;
        self.search
            .list_docs_page(&meta.id, limit.clamp(1, 200), offset)
            .await
    }

    pub async fn upsert_search_doc(
        &self,
        caller: &Caller,
        id_or_name: &str,
        doc: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let meta = self.resolve_search_index(id_or_name).await?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("search index '{}'", meta.name),
        )?;
        self.search.upsert_doc(&meta.id, doc).await
    }

    pub async fn delete_search_doc(
        &self,
        caller: &Caller,
        id_or_name: &str,
        doc_id: &str,
    ) -> Result<bool> {
        let meta = self.resolve_search_index(id_or_name).await?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("search index '{}'", meta.name),
        )?;
        self.search.delete_doc(&meta.id, doc_id).await
    }

    pub async fn query_search_index(
        &self,
        caller: &Caller,
        id_or_name: &str,
        input: SearchQueryInput,
    ) -> Result<SearchQueryResult> {
        let meta = self.resolve_search_index(id_or_name).await?;
        Self::require_resource_read(
            caller,
            meta.project_id.as_deref(),
            &format!("search index '{}'", meta.name),
        )?;
        self.search.search(&meta.id, input).await
    }
}
