mod backpressure;
mod router;
mod spine;
mod transformer;
pub mod observability;

pub use backpressure::{BackpressureGate, BackpressureState};
pub use observability::{
    AlertBuffer, AlertEvent, AlertSeverity, HealthCheckResult, HealthStatus, MetricSample,
    Observability, ObservabilityEvent, ObservabilityKind, ProfileSpan, Profiler,
};
pub use router::{EventLane, EventLaneHub};
pub use spine::{
    DeadLetterReason, DeadLetterRecord, EventFilter, EventPublishContext, FilteredReceiver,
};
pub use transformer::{SpineTransformer, SpineTransformerRegistry};

use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, RwLock as StdRwLock,
    },
};
use tokio::sync::RwLock;
use xuanyu_protocol::{AppEvent, Envelope};

pub const DEFAULT_REPLAY_BUFFER: usize = 4096;

pub type JournalSink = Arc<dyn Fn(&StoredEvent) + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredEvent {
    pub sequence: u64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventReplayBatch {
    pub last_sequence: u64,
    pub next_sequence: u64,
    pub overflow: bool,
    pub events: Vec<StoredEvent>,
}

#[derive(Clone)]
pub struct EventBus {
    sender: tokio::sync::broadcast::Sender<Envelope<AppEvent>>,
    lanes: EventLaneHub,
    sequence: Arc<AtomicU64>,
    journal: Arc<RwLock<VecDeque<StoredEvent>>>,
    journal_capacity: usize,
    dead_letters: Arc<RwLock<VecDeque<DeadLetterRecord>>>,
    journal_sink: Option<JournalSink>,
    backpressure: Arc<Mutex<BackpressureGate>>,
    transformers: Arc<StdRwLock<SpineTransformerRegistry>>,
}

impl EventBus {
    pub fn new(buffer: usize) -> Self {
        Self::with_replay_buffer(buffer, DEFAULT_REPLAY_BUFFER)
    }

    pub fn with_replay_buffer(live_buffer: usize, replay_buffer: usize) -> Self {
        let (sender, _) = tokio::sync::broadcast::channel(live_buffer.max(16));
        Self {
            sender,
            lanes: EventLaneHub::new(),
            sequence: Arc::new(AtomicU64::new(0)),
            journal: Arc::new(RwLock::new(VecDeque::with_capacity(replay_buffer.min(256)))),
            journal_capacity: replay_buffer.max(64),
            dead_letters: Arc::new(RwLock::new(VecDeque::with_capacity(128))),
            journal_sink: None,
            backpressure: Arc::new(Mutex::new(BackpressureGate::new(
                replay_buffer.max(64),
                None,
            ))),
            transformers: Arc::new(StdRwLock::new(SpineTransformerRegistry::new())),
        }
    }

    pub fn set_journal_sink(&mut self, sink: JournalSink) {
        self.journal_sink = Some(sink);
    }

    pub fn with_journal_sink(mut self, sink: JournalSink) -> Self {
        self.journal_sink = Some(sink);
        self
    }

    pub fn publish(&self, event: AppEvent) {
        self.publish_scoped(EventPublishContext::from_event(&event), event);
    }

    pub fn publish_scoped(&self, ctx: EventPublishContext, event: AppEvent) {
        let journal_depth = self
            .journal
            .try_read()
            .map(|journal| journal.len())
            .unwrap_or(0);
        let high_watermark = self
            .backpressure
            .lock()
            .expect("backpressure lock")
            .state()
            .high_watermark;
        if journal_depth >= high_watermark {
            if let Ok(mut letters) = self.dead_letters.try_write() {
                letters.push_back(DeadLetterRecord {
                    reason: DeadLetterReason::FilterMismatch,
                    detail: format!(
                        "backpressure: journal depth {journal_depth} >= high watermark {high_watermark}"
                    ),
                    created_at: chrono::Utc::now(),
                });
            }
            return;
        }

        let lane = EventLane::classify(&event);
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let mut envelope = Envelope::new("xuanyu.event", "core", "ui", event);
        envelope
            .meta
            .insert("sequence".to_string(), sequence.to_string());
        envelope
            .meta
            .insert("lane".to_string(), lane.as_str().to_string());
        if let Some(session_id) = ctx.session_id.as_ref() {
            envelope
                .meta
                .insert("sessionId".to_string(), session_id.clone());
        }
        if let Some(turn_id) = ctx.turn_id.as_ref() {
            envelope.meta.insert("turnId".to_string(), turn_id.clone());
        }
        if let Some(trace_id) = ctx.trace_id.as_ref() {
            envelope.meta.insert("traceId".to_string(), trace_id.clone());
        }
        if let Ok(registry) = self.transformers.read() {
            envelope = registry.apply(envelope);
        }
        let stored = StoredEvent {
            sequence,
            timestamp: envelope.timestamp,
            kind: envelope.kind.clone(),
            session_id: ctx.session_id.clone(),
            turn_id: ctx.turn_id.clone(),
            lane: Some(lane.as_str().to_string()),
            payload: serde_json::to_value(&envelope.payload).unwrap_or(serde_json::Value::Null),
        };
        if let Ok(mut journal) = self.journal.try_write() {
            if journal.len() >= self.journal_capacity {
                journal.pop_front();
            }
            journal.push_back(stored.clone());
        }
        if let Some(sink) = &self.journal_sink {
            sink(&stored);
        }
        self.lanes.publish(lane, envelope.clone());
        if lane != EventLane::Global {
            self.lanes.publish(EventLane::Global, envelope.clone());
        }
        if self.sender.send(envelope).is_err() {
            tracing::debug!("eventbus publish skipped because there are no subscribers");
        }
    }

    pub fn backpressure_state(&self) -> BackpressureState {
        self.backpressure
            .lock()
            .expect("backpressure lock")
            .state()
    }

    pub fn register_transformer(
        &self,
        name: impl Into<String>,
        transformer: SpineTransformer,
    ) {
        if let Ok(mut registry) = self.transformers.write() {
            registry.register(name, transformer);
        }
    }

    pub fn subscribe_lane(&self, lane: EventLane) -> tokio::sync::broadcast::Receiver<Envelope<AppEvent>> {
        self.lanes.subscribe(lane)
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Envelope<AppEvent>> {
        self.sender.subscribe()
    }

    pub fn subscribe_filtered(&self, filter: EventFilter) -> FilteredReceiver {
        FilteredReceiver::new(self.sender.subscribe(), filter, self.dead_letters.clone())
    }

    pub fn next_sequence(&self) -> u64 {
        self.sequence.load(Ordering::Relaxed)
    }

    pub fn restore_sequence(&self, sequence: u64) {
        self.sequence.store(sequence, Ordering::Relaxed);
    }

    pub async fn replay_since(&self, last_sequence: u64) -> EventReplayBatch {
        self.replay_since_filtered(last_sequence, EventFilter::default())
            .await
    }

    pub async fn replay_since_filtered(
        &self,
        last_sequence: u64,
        filter: EventFilter,
    ) -> EventReplayBatch {
        let journal = self.journal.read().await;
        let next_sequence = self.next_sequence();
        let oldest = journal.front().map(|item| item.sequence);
        let overflow = oldest
            .map(|oldest| last_sequence > 0 && last_sequence < oldest.saturating_sub(1))
            .unwrap_or(journal.is_empty() && last_sequence > 0);
        let events = journal
            .iter()
            .filter(|item| last_sequence == 0 || item.sequence > last_sequence)
            .filter(|item| filter.matches_stored(item))
            .cloned()
            .collect::<Vec<_>>();
        EventReplayBatch {
            last_sequence,
            next_sequence,
            overflow,
            events,
        }
    }

    pub async fn dead_letters(&self, limit: usize) -> Vec<DeadLetterRecord> {
        let limit = limit.clamp(1, 512);
        let letters = self.dead_letters.read().await;
        letters.iter().rev().take(limit).cloned().collect()
    }
}
