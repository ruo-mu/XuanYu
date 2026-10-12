use anyhow::{anyhow, Context};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{env, time::Duration};

use super::{embed_text, MEMORY_VECTOR_SIZE};

const DEFAULT_OPENAI_EMBEDDING_MODEL: &str = "text-embedding-3-small";
const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_OPENAI_DIMENSIONS: usize = 1536;
const MAX_EMBEDDING_DIMENSIONS: usize = 4096;
const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_BATCH_SIZE: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryEmbeddingProviderKind {
    Openai,
    Local,
    Hash,
}

impl MemoryEmbeddingProviderKind {
    fn from_value(value: Option<&Value>) -> Self {
        match value.and_then(Value::as_str).unwrap_or_default().trim() {
            "openai" => Self::Openai,
            "local" => Self::Local,
            _ => Self::Hash,
        }
    }

    fn uses_external_api(self) -> bool {
        matches!(self, Self::Openai | Self::Local)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEmbeddingConfig {
    pub provider: MemoryEmbeddingProviderKind,
    pub model: String,
    pub dimensions: usize,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub timeout_ms: u64,
    pub batch_size: usize,
}

impl Default for MemoryEmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: MemoryEmbeddingProviderKind::Hash,
            model: "hash-fnv-bow".to_string(),
            dimensions: MEMORY_VECTOR_SIZE,
            api_key: None,
            base_url: None,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            batch_size: DEFAULT_BATCH_SIZE,
        }
    }
}

impl MemoryEmbeddingConfig {
    pub fn from_app_config(config: &Value) -> Self {
        let Some(embedding) = config
            .get("memory")
            .and_then(Value::as_object)
            .and_then(|memory| memory.get("embedding"))
        else {
            return Self::default();
        };
        Self::from_value(embedding)
    }

    pub fn from_value(value: &Value) -> Self {
        let provider = MemoryEmbeddingProviderKind::from_value(value.get("provider"));
        let default_dimensions = if provider == MemoryEmbeddingProviderKind::Hash {
            MEMORY_VECTOR_SIZE
        } else {
            DEFAULT_OPENAI_DIMENSIONS
        };
        let model = value
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| match provider {
                MemoryEmbeddingProviderKind::Hash => "hash-fnv-bow".to_string(),
                _ => DEFAULT_OPENAI_EMBEDDING_MODEL.to_string(),
            });
        let api_key = value
            .get("apiKey")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_string)
            .or_else(|| {
                if provider == MemoryEmbeddingProviderKind::Openai {
                    env::var("OPENAI_API_KEY")
                        .ok()
                        .map(|key| key.trim().to_string())
                        .filter(|key| !key.is_empty())
                } else {
                    None
                }
            });
        let base_url = value
            .get("baseUrl")
            .and_then(Value::as_str)
            .map(normalize_base_url)
            .filter(|url| !url.is_empty())
            .or_else(|| {
                if provider == MemoryEmbeddingProviderKind::Openai {
                    Some(DEFAULT_OPENAI_BASE_URL.to_string())
                } else {
                    None
                }
            });
        Self {
            provider,
            model,
            dimensions: normalize_dimensions(value.get("dimensions"), default_dimensions),
            api_key,
            base_url,
            timeout_ms: normalize_timeout_ms(value.get("timeoutMs")),
            batch_size: normalize_batch_size(value.get("batchSize")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEmbeddingProviderStatus {
    pub provider: MemoryEmbeddingProviderKind,
    pub model: String,
    pub dimensions: usize,
    pub uses_external_api: bool,
    pub configured: bool,
    pub fallback: String,
    pub base_url: Option<String>,
    pub issue: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MemoryEmbeddingProvider {
    config: MemoryEmbeddingConfig,
    client: Client,
}

impl MemoryEmbeddingProvider {
    pub fn new(config: MemoryEmbeddingConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            config,
            client,
        }
    }

    /// When config still uses hash fallback but the inference worker exposes embedding,
    /// promote the reported provider to local and attach the worker bridge.
    pub fn from_app_config(config: &Value) -> Self {
        Self::new(MemoryEmbeddingConfig::from_app_config(config))
    }

    pub fn config(&self) -> &MemoryEmbeddingConfig {
        &self.config
    }

    pub fn uses_external_api(&self) -> bool {
        self.config.provider.uses_external_api()
    }

    pub fn status(&self) -> MemoryEmbeddingProviderStatus {
        let issue = match self.config.provider {
            MemoryEmbeddingProviderKind::Hash if false => None,
            MemoryEmbeddingProviderKind::Hash => None,
            MemoryEmbeddingProviderKind::Openai if self.config.api_key.is_none() => Some(
                "OpenAI embedding apiKey is not configured; search falls back to hash.".to_string(),
            ),
            MemoryEmbeddingProviderKind::Local
                if self.config.base_url.is_none() =>
            {
                Some(
                    "Local embedding baseUrl is not configured; search falls back to hash.".to_string(),
                )
            }
            _ => None,
        };
        MemoryEmbeddingProviderStatus {
            provider: self.config.provider,
            model: self.config.model.clone(),
            dimensions: self.config.dimensions,
            uses_external_api: self.uses_external_api(),
            configured: issue.is_none(),
            fallback: if false {
                "inference-worker-embedding".to_string()
            } else {
                "hash-fnv-bow".to_string()
            },
            base_url: self.config.base_url.clone(),
            issue,
        }
    }

    pub async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let vectors = self.embed_batch(&[text.to_string()]).await?;
        vectors
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("embedding provider returned no vectors"))
    }

    pub async fn embed_batch(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        if self.config.provider == MemoryEmbeddingProviderKind::Hash {
            return Ok(self.hash_batch(texts, self.config.dimensions));
        }
        let mut vectors = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(self.config.batch_size.max(1)) {
            vectors.extend(self.request_embeddings(chunk).await?);
        }
        Ok(vectors)
    }

    pub async fn embed_batch_or_hash(&self, texts: &[String]) -> Vec<Vec<f32>> {
        match self.embed_batch(texts).await {
            Ok(vectors) => vectors,
            Err(_) => self.hash_batch(texts, MEMORY_VECTOR_SIZE),
        }
    }

    fn hash_batch(&self, texts: &[String], dimensions: usize) -> Vec<Vec<f32>> {
        texts
            .iter()
            .map(|text| embed_text(text, dimensions.max(1)))
            .collect()
    }

    async fn request_embeddings(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let base_url = self
            .config
            .base_url
            .as_deref()
            .filter(|url| !url.is_empty())
            .ok_or_else(|| anyhow!("local embedding baseUrl is not configured"))?;
        let api_key = self
            .config
            .api_key
            .as_deref()
            .or_else(|| {
                if self.config.provider == MemoryEmbeddingProviderKind::Local {
                    Some("local")
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow!("OpenAI embedding apiKey is not configured"))?;
        let response = self
            .client
            .post(format!("{base_url}/embeddings"))
            .bearer_auth(api_key)
            .json(&json!({
                "model": self.config.model,
                "input": texts,
                "dimensions": self.config.dimensions
            }))
            .send()
            .await
            .with_context(|| format!("embedding request failed for {base_url}/embeddings"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("failed to read embedding response body")?;
        if !status.is_success() {
            return Err(anyhow!("embedding request failed: {status} {body}"));
        }
        let payload: EmbeddingResponse =
            serde_json::from_str(&body).context("embedding response is not valid JSON")?;
        normalize_embedding_response(payload, texts.len(), self.config.dimensions)
    }
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    #[serde(default)]
    data: Vec<EmbeddingRecord>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingRecord {
    embedding: Option<Vec<f32>>,
    index: Option<usize>,
}

fn normalize_embedding_response(
    payload: EmbeddingResponse,
    expected_count: usize,
    dimensions: usize,
) -> anyhow::Result<Vec<Vec<f32>>> {
    let mut ordered = payload
        .data
        .into_iter()
        .enumerate()
        .map(|(fallback_index, item)| (item.index.unwrap_or(fallback_index), item.embedding))
        .collect::<Vec<_>>();
    ordered.sort_by_key(|(index, _)| *index);
    if ordered.len() != expected_count {
        return Err(anyhow!(
            "embedding response count {} does not match input count {}",
            ordered.len(),
            expected_count
        ));
    }
    ordered
        .into_iter()
        .map(|(_, vector)| {
            vector
                .as_deref()
                .map(|vector| normalize_vector(vector, dimensions))
                .ok_or_else(|| anyhow!("embedding response is missing vector data"))
        })
        .collect()
}

fn normalize_vector(vector: &[f32], dimensions: usize) -> Vec<f32> {
    let mut resized = vec![0.0; dimensions.max(1)];
    for (idx, value) in vector.iter().take(resized.len()).enumerate() {
        resized[idx] = if value.is_finite() { *value } else { 0.0 };
    }
    let norm = resized
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if norm > 0.0 {
        for value in &mut resized {
            *value /= norm;
        }
    }
    resized
}

fn normalize_base_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

fn normalize_timeout_ms(value: Option<&Value>) -> u64 {
    value
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(1_000, 60_000)
}

fn normalize_batch_size(value: Option<&Value>) -> usize {
    value
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_BATCH_SIZE)
        .clamp(1, 128)
}

fn normalize_dimensions(value: Option<&Value>, default_dimensions: usize) -> usize {
    value
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(default_dimensions)
        .clamp(1, MAX_EMBEDDING_DIMENSIONS)
}
