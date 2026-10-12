use std::collections::BTreeMap;

use xuanyu_protocol::{AppEvent, Envelope};

pub type SpineTransformer = Box<dyn Fn(Envelope<AppEvent>) -> Envelope<AppEvent> + Send + Sync>;

/// Mirrors `spine/transformers/index.ts` registry (minimal).
pub struct SpineTransformerRegistry {
    transformers: BTreeMap<String, SpineTransformer>,
}

impl Default for SpineTransformerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SpineTransformerRegistry {
    pub fn new() -> Self {
        Self {
            transformers: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, name: impl Into<String>, transformer: SpineTransformer) {
        self.transformers.insert(name.into(), transformer);
    }

    pub fn apply(&self, envelope: Envelope<AppEvent>) -> Envelope<AppEvent> {
        let mut envelope = envelope;
        for transformer in self.transformers.values() {
            envelope = transformer(envelope);
        }
        envelope
    }
}
