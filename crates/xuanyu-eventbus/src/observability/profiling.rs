use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSpan {
    pub id: String,
    pub name: String,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
}

const MAX_SPANS: usize = 2000;

#[derive(Debug, Default)]
pub struct Profiler {
    spans: Vec<ProfileSpan>,
}

impl Profiler {
    pub fn start(&mut self, name: impl Into<String>) -> String {
        let span = ProfileSpan {
            id: format!("span_{}", Uuid::new_v4()),
            name: name.into(),
            started_at: UtcMillis::now(),
            ended_at: None,
            duration_ms: None,
        };
        let id = span.id.clone();
        self.spans.push(span);
        if self.spans.len() > MAX_SPANS {
            let drain = self.spans.len().saturating_sub(MAX_SPANS);
            self.spans.drain(0..drain);
        }
        id
    }

    pub fn finish(&mut self, span_id: &str) -> Option<ProfileSpan> {
        let ended_at = UtcMillis::now();
        let span = self.spans.iter_mut().find(|span| span.id == span_id)?;
        span.ended_at = Some(ended_at);
        span.duration_ms = Some(ended_at.saturating_sub(span.started_at));
        Some(span.clone())
    }

    pub fn list(&self) -> Vec<ProfileSpan> {
        self.spans.clone()
    }

    pub fn recent(&self, limit: usize) -> Vec<ProfileSpan> {
        self.spans
            .iter()
            .rev()
            .take(limit.max(1))
            .cloned()
            .collect()
    }
}

struct UtcMillis;

impl UtcMillis {
    fn now() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}
