use serde::{Deserialize, Serialize};

/// Mirrors `spine/backpressure/index.ts` `BackpressureGate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackpressureState {
    pub accepting: bool,
    pub current_depth: usize,
    pub high_watermark: usize,
    pub low_watermark: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub struct BackpressureGate {
    high_watermark: usize,
    low_watermark: usize,
    current_depth: usize,
    forced_reason: Option<String>,
}

impl BackpressureGate {
    pub fn new(high_watermark: usize, low_watermark: Option<usize>) -> Self {
        let high_watermark = high_watermark.max(1);
        let low_watermark = low_watermark
            .unwrap_or(high_watermark / 2)
            .min(high_watermark.saturating_sub(1));
        Self {
            high_watermark,
            low_watermark,
            current_depth: 0,
            forced_reason: None,
        }
    }

    pub fn state(&self) -> BackpressureState {
        let accepting = self.forced_reason.is_none() && self.current_depth < self.high_watermark;
        BackpressureState {
            accepting,
            current_depth: self.current_depth,
            high_watermark: self.high_watermark,
            low_watermark: self.low_watermark,
            reason: self.forced_reason.clone().or_else(|| {
                if accepting {
                    None
                } else {
                    Some("high_watermark_reached".to_string())
                }
            }),
        }
    }

    pub fn enter(&mut self, count: usize) -> BackpressureState {
        self.current_depth = self.current_depth.saturating_add(count);
        self.state()
    }

    pub fn leave(&mut self, count: usize) -> BackpressureState {
        self.current_depth = self.current_depth.saturating_sub(count);
        if self.current_depth <= self.low_watermark {
            self.forced_reason = None;
        }
        self.state()
    }

    pub fn reject(&mut self, reason: impl Into<String>) -> BackpressureState {
        self.forced_reason = Some(reason.into());
        self.state()
    }

    pub fn can_accept(&self) -> bool {
        self.state().accepting
    }
}
