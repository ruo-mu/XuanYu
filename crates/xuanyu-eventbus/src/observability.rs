use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use tokio::sync::RwLock;
use uuid::Uuid;

mod alerts;
mod profiling;

pub use alerts::{AlertBuffer, AlertEvent, AlertSeverity};
pub use profiling::{ProfileSpan, Profiler};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservabilityKind {
    Log,
    Metric,
    Trace,
    Audit,
    Alert,
    Health,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservabilityEvent {
    pub id: String,
    pub kind: ObservabilityKind,
    pub payload: serde_json::Value,
    pub trace_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricSample {
    pub name: String,
    pub value: f64,
    pub unit: String,
    pub tags: BTreeMap<String, String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ok,
    Degraded,
    Down,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheckResult {
    pub component: String,
    pub status: HealthStatus,
    pub detail: Option<String>,
    pub checked_at: DateTime<Utc>,
}

pub struct Observability {
    capacity: usize,
    events: RwLock<VecDeque<ObservabilityEvent>>,
    metrics: RwLock<BTreeMap<String, MetricSample>>,
    health: RwLock<BTreeMap<String, HealthCheckResult>>,
    alerts: RwLock<AlertBuffer>,
    profiler: RwLock<Profiler>,
}

impl Observability {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            events: RwLock::new(VecDeque::new()),
            metrics: RwLock::new(BTreeMap::new()),
            health: RwLock::new(BTreeMap::new()),
            alerts: RwLock::new(AlertBuffer::default()),
            profiler: RwLock::new(Profiler::default()),
        }
    }

    pub async fn record(&self, mut event: ObservabilityEvent) {
        if event.id.is_empty() {
            event.id = new_id("obs");
        }
        let mut events = self.events.write().await;
        while events.len() >= self.capacity {
            events.pop_front();
        }
        events.push_back(event);
    }

    pub async fn log(&self, level: &str, message: impl Into<String>) {
        self.record(ObservabilityEvent {
            id: new_id("log"),
            kind: ObservabilityKind::Log,
            payload: serde_json::json!({ "level": level, "message": message.into() }),
            trace_id: None,
            created_at: Utc::now(),
        })
        .await;
    }

    pub async fn metric(&self, name: impl Into<String>, value: f64, unit: impl Into<String>) {
        let sample = MetricSample {
            name: name.into(),
            value,
            unit: unit.into(),
            tags: BTreeMap::new(),
            updated_at: Utc::now(),
        };
        self.metrics
            .write()
            .await
            .insert(sample.name.clone(), sample.clone());
        self.record(ObservabilityEvent {
            id: new_id("metric"),
            kind: ObservabilityKind::Metric,
            payload: serde_json::to_value(sample).expect("metric sample serializes"),
            trace_id: None,
            created_at: Utc::now(),
        })
        .await;
    }

    pub async fn health(
        &self,
        component: impl Into<String>,
        status: HealthStatus,
        detail: Option<String>,
    ) {
        let result = HealthCheckResult {
            component: component.into(),
            status,
            detail,
            checked_at: Utc::now(),
        };
        self.health
            .write()
            .await
            .insert(result.component.clone(), result.clone());
        self.record(ObservabilityEvent {
            id: new_id("health"),
            kind: ObservabilityKind::Health,
            payload: serde_json::to_value(result).expect("health result serializes"),
            trace_id: None,
            created_at: Utc::now(),
        })
        .await;
    }

    pub async fn alert(
        &self,
        severity: AlertSeverity,
        title: impl Into<String>,
        payload: Option<serde_json::Value>,
        trace_id: Option<String>,
    ) -> AlertEvent {
        let event = self
            .alerts
            .write()
            .await
            .push(severity, title, payload, trace_id);
        self.record(ObservabilityEvent {
            id: new_id("alert"),
            kind: ObservabilityKind::Alert,
            payload: serde_json::to_value(&event).expect("alert serializes"),
            trace_id: event.trace_id.clone(),
            created_at: event.created_at,
        })
        .await;
        event
    }

    pub async fn alerts(&self, limit: usize) -> Vec<AlertEvent> {
        self.alerts.read().await.recent(limit.max(1))
    }

    pub async fn start_profile_span(&self, name: impl Into<String>) -> String {
        self.profiler.write().await.start(name)
    }

    pub async fn finish_profile_span(&self, span_id: &str) -> Option<ProfileSpan> {
        let span = self.profiler.write().await.finish(span_id);
        if let Some(span) = span.clone() {
            self.record(ObservabilityEvent {
                id: new_id("profile"),
                kind: ObservabilityKind::Trace,
                payload: serde_json::to_value(span).expect("profile span serializes"),
                trace_id: None,
                created_at: Utc::now(),
            })
            .await;
        }
        span
    }

    pub async fn profile_spans(&self, limit: usize) -> Vec<ProfileSpan> {
        self.profiler.read().await.recent(limit.max(1))
    }

    pub async fn recent_events(&self, limit: usize) -> Vec<ObservabilityEvent> {
        self.events
            .read()
            .await
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    pub async fn metrics(&self) -> Vec<MetricSample> {
        self.metrics.read().await.values().cloned().collect()
    }

    pub async fn health_summary(&self) -> Vec<HealthCheckResult> {
        self.health.read().await.values().cloned().collect()
    }
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4())
}
