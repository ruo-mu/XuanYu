use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Progressive loop detection for repeated tool-call stagnation.
const MAX_STAGNATION: usize = 4;
const MAX_REFLECTIONS: usize = 3;
const RECENT_WINDOW: usize = 12;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TurnFingerprint {
    pub tool_names: Vec<String>,
    pub output_prefix: String,
    pub arg_shapes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopIntervention {
    None,
    Nudge,
    Reflect,
    ForceStop,
}

#[derive(Debug, Default)]
pub struct ToolLoopGuard {
    recent_turns: Vec<TurnFingerprint>,
    reflection_attempts: usize,
    consecutive_no_progress: usize,
    turns_since_last_intervention: usize,
    last_tool_call_count: isize,
}

impl ToolLoopGuard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_tool_iteration(
        &mut self,
        tool_calls: &[(&str, &Value)],
        output_text: &str,
        context_pressure: f64,
    ) -> LoopIntervention {
        let fingerprint = TurnFingerprint {
            tool_names: tool_calls.iter().map(|(name, _)| (*name).to_string()).collect(),
            output_prefix: output_text.chars().take(200).collect::<String>().trim().to_string(),
            arg_shapes: tool_calls
                .iter()
                .map(|(_, args)| arg_shape(args))
                .collect(),
        };
        self.recent_turns.push(fingerprint.clone());
        while self.recent_turns.len() > RECENT_WINDOW {
            self.recent_turns.remove(0);
        }

        let pattern_window = self
            .recent_turns
            .iter()
            .rev()
            .take(MAX_STAGNATION)
            .cloned()
            .collect::<Vec<_>>();
        let pattern_window: Vec<_> = pattern_window.into_iter().rev().collect();
        let tool_pattern_stale = pattern_window.len() >= MAX_STAGNATION
            && pattern_window.iter().all(|turn| {
                turn.tool_names == pattern_window[0].tool_names
                    && turn.arg_shapes == pattern_window[0].arg_shapes
            });

        let output_stale = self.recent_turns.len() >= MAX_STAGNATION
            && self
                .recent_turns
                .iter()
                .rev()
                .take(MAX_STAGNATION)
                .collect::<Vec<_>>()
                .windows(2)
                .all(|pair| text_similarity(&pair[1].output_prefix, &pair[0].output_prefix) > 0.85);

        let prev = self.recent_turns.len().saturating_sub(2);
        let prev_turn = prev.checked_sub(0).and_then(|idx| self.recent_turns.get(idx));
        let tool_names_changed = prev_turn
            .map(|prev| {
                fingerprint.tool_names.join(",") != prev.tool_names.join(",")
            })
            .unwrap_or(true);
        let output_changed = prev_turn
            .map(|prev| fingerprint.output_prefix != prev.output_prefix)
            .unwrap_or(true);
        let made_progress = tool_names_changed
            || output_changed
            || tool_calls.len() as isize != self.last_tool_call_count;
        if !made_progress {
            self.consecutive_no_progress += 1;
        } else {
            self.consecutive_no_progress = 0;
        }
        self.last_tool_call_count = tool_calls.len() as isize;

        let is_stuck =
            tool_pattern_stale || output_stale || self.consecutive_no_progress >= 3;
        let intervention = if is_stuck {
            if self.reflection_attempts >= MAX_REFLECTIONS {
                LoopIntervention::ForceStop
            } else if self.reflection_attempts >= 1 || context_pressure > 0.85 {
                LoopIntervention::Reflect
            } else {
                LoopIntervention::Nudge
            }
        } else {
            LoopIntervention::None
        };

        match intervention {
            LoopIntervention::Nudge | LoopIntervention::Reflect => {
                self.reflection_attempts += 1;
                self.consecutive_no_progress = 0;
                self.recent_turns.clear();
                self.turns_since_last_intervention = 0;
            }
            LoopIntervention::ForceStop => {}
            LoopIntervention::None => {
                if self.reflection_attempts > 0 {
                    self.turns_since_last_intervention += 1;
                    if self.turns_since_last_intervention >= 3 {
                        self.reflection_attempts = self.reflection_attempts.saturating_sub(1);
                        self.turns_since_last_intervention = 0;
                    }
                }
                if made_progress {
                    self.consecutive_no_progress = 0;
                }
            }
        }

        intervention
    }

    pub fn intervention_message(
        intervention: LoopIntervention,
        fingerprint: &TurnFingerprint,
        context_pressure: f64,
    ) -> Option<String> {
        match intervention {
            LoopIntervention::None | LoopIntervention::ForceStop => None,
            LoopIntervention::Nudge => {
                let mut hints = vec!["[System] 检测到可能的重复模式。".to_string()];
                let names = fingerprint
                    .tool_names
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                if !names.is_empty() {
                    hints.push(format!(
                        "最近 {MAX_STAGNATION} 轮使用了相同工具序列（{names}）。"
                    ));
                }
                hints.push("请确认：当前方法是否有效？如果任务已完成，请直接回复结果。".to_string());
                hints.push(
                    "如果确实需要继续，请在回复中简要说明下一步方向。".to_string(),
                );
                Some(hints.join("\n"))
            }
            LoopIntervention::Reflect => {
                let mut hints = vec!["[System] 重复模式持续存在，请认真反思当前策略。".to_string()];
                hints.push("相同的工具调用序列已重复多轮，请换一种方式完成目标。".to_string());
                if context_pressure > 0.85 {
                    hints.push("上下文窗口使用率已超过 85%，请在 1-2 轮内给出最终回复。".to_string());
                }
                hints.extend([
                    "你可以：".to_string(),
                    "1. 如果任务已完成或接近完成，直接给出最终回复（不要调用工具）".to_string(),
                    "2. 如果卡住了，尝试完全不同的方法".to_string(),
                    "3. 如果确实无法完成，请诚实说明原因和已完成的部分".to_string(),
                ]);
                Some(hints.join("\n"))
            }
        }
    }
}

/// Compact argument shape fingerprint for loop detection.
pub fn arg_shape(args: &Value) -> String {
    let Some(object) = args.as_object() else {
        let raw = args.to_string();
        return format!("raw[{}]", raw.len());
    };
    let mut keys = object.keys().collect::<Vec<_>>();
    keys.sort();
    let shape_parts = keys
        .into_iter()
        .map(|key| {
            let bucket = match object.get(key) {
                Some(Value::String(value)) => {
                    if value.is_empty() {
                        "empty"
                    } else if value.len() < 100 {
                        "short"
                    } else if value.len() < 1000 {
                        "medium"
                    } else {
                        "long"
                    }
                }
                Some(Value::Number(_)) => "number",
                Some(Value::Bool(_)) => "bool",
                Some(Value::Null) => "null",
                Some(Value::Array(items)) => return format!("{key}:array[{}]", items.len()),
                Some(Value::Object(_)) => "object",
                None => "null",
            };
            format!("{key}:{bucket}")
        })
        .collect::<Vec<_>>();
    shape_parts.join("|")
}

/// Mixed char/word n-gram Jaccard similarity for near-duplicate tool text.
pub fn text_similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let a_ngrams = ngrams(a);
    let b_ngrams = ngrams(b);
    let intersection = a_ngrams.intersection(&b_ngrams).count();
    let union = a_ngrams.len() + b_ngrams.len() - intersection;
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

fn ngrams(text: &str) -> std::collections::BTreeSet<String> {
    let mut ngrams = std::collections::BTreeSet::new();
    let chars = text.chars().take(800).collect::<Vec<_>>();
    for window in chars.windows(2) {
        ngrams.insert(window.iter().collect());
    }
    for window in chars.windows(3) {
        if let Some(first) = window.first() {
            if ('\u{4e00}'..='\u{9fff}').contains(first) {
                ngrams.insert(window.iter().collect());
            }
        }
    }
    ngrams
}
