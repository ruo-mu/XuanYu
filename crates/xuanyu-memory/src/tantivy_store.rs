use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Schema, Value, STORED, STRING, TEXT};
use tantivy::{doc, Index, IndexWriter, ReloadPolicy, TantivyDocument};

use crate::{kind_for_category, MemoryEntry, MemorySearchQuery, MemorySearchResult, NewMemoryEntry};

#[derive(Debug, Clone)]
pub struct TantivyMemoryConfig {
    pub index_path: PathBuf,
}

pub struct TantivyMemoryStore {
    index: Index,
    id_field: tantivy::schema::Field,
    content_field: tantivy::schema::Field,
    category_field: tantivy::schema::Field,
    writer: Mutex<IndexWriter>,
}

impl TantivyMemoryStore {
    pub fn open(config: TantivyMemoryConfig) -> Result<Self> {
        std::fs::create_dir_all(&config.index_path)
            .with_context(|| format!("create tantivy dir {}", config.index_path.display()))?;
        let mut schema_builder = Schema::builder();
        let id_field = schema_builder.add_text_field("id", STRING | STORED);
        let content_field = schema_builder.add_text_field("content", TEXT | STORED);
        let category_field = schema_builder.add_text_field("category", STRING | STORED);
        let schema = schema_builder.build();
        let index = Index::open_or_create(
            tantivy::directory::MmapDirectory::open(&config.index_path)?,
            schema.clone(),
        )?;
        let writer = index.writer(15_000_000)?;
        Ok(Self {
            index,
            id_field,
            content_field,
            category_field,
            writer: Mutex::new(writer),
        })
    }

    pub fn upsert(&self, entry: &NewMemoryEntry, id: &str) -> Result<()> {
        let mut writer = self.writer.lock().map_err(|_| anyhow::anyhow!("tantivy lock"))?;
        writer.delete_term(tantivy::Term::from_field_text(self.id_field, id));
        writer.add_document(doc!(
            self.id_field => id,
            self.content_field => entry.content.as_str(),
            self.category_field => entry.category.as_str(),
        ))?;
        writer.commit()?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let mut writer = self.writer.lock().map_err(|_| anyhow::anyhow!("tantivy lock"))?;
        writer.delete_term(tantivy::Term::from_field_text(self.id_field, id));
        writer.commit()?;
        Ok(())
    }

    pub fn search(&self, query: &MemorySearchQuery, limit: usize) -> Result<Vec<MemorySearchResult>> {
        let Some(text) = query.text.as_ref().filter(|value| !value.trim().is_empty()) else {
            return Ok(Vec::new());
        };
        let reader = self
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        let searcher = reader.searcher();
        let parser = QueryParser::for_index(&self.index, vec![self.content_field]);
        let parsed = parser.parse_query(text)?;
        let top_docs = searcher.search(&parsed, &TopDocs::with_limit(limit.max(1)))?;
        Ok(top_docs
            .into_iter()
            .filter_map(|(score, doc_address)| {
                let doc: TantivyDocument = searcher.doc(doc_address).ok()?;
                let id = doc.get_first(self.id_field)?.as_str()?.to_string();
                let content = doc.get_first(self.content_field)?.as_str()?.to_string();
                let category = doc
                    .get_first(self.category_field)
                    .and_then(|value| value.as_str())
                    .unwrap_or("general")
                    .to_string();
                Some(MemorySearchResult {
                    entry: MemoryEntry {
                        id,
                        kind: kind_for_category(&category),
                        category,
                        content,
                        tags: Vec::new(),
                        context: query.session_id.clone(),
                        importance: 0.5,
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                        last_accessed_at: None,
                        access_count: 0,
                    },
                    score,
                    reasons: vec!["tantivy_fts".to_string()],
                })
            })
            .collect())
    }
}

pub fn tantivy_memory_config_from_value(value: &serde_json::Value, data_dir: &Path) -> Option<TantivyMemoryConfig> {
    let memory = value.get("memory")?;
    let tantivy = memory.get("tantivy").or_else(|| memory.get("local"))?;
    let index_path = tantivy
        .get("indexPath")
        .or_else(|| tantivy.get("index_path"))
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.join("memory").join("tantivy"));
    Some(TantivyMemoryConfig { index_path })
}
