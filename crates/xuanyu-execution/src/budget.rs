use crate::token_counter::{estimate_tokens, max_input_tokens, TokenCounterConfig, TokenCountBreakdown};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextBudgetSection {
    pub id: String,
    pub section_type: String,
    pub title: String,
    pub priority: u8,
    pub allocated_tokens: u64,
    pub used_tokens: u64,
    pub dropped: bool,
    pub truncated: bool,
    pub loaded: bool,
    pub rendered_text: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextBudgetAllocation {
    pub sections: Vec<ContextBudgetSection>,
    pub max_input_tokens: u64,
    pub tool_schema_tokens: u64,
    pub used_input_tokens: u64,
    pub remaining_input_tokens: u64,
    pub dropped_message_count: u64,
    pub truncated_message_count: u64,
    pub breakdown: TokenCountBreakdown,
}

#[derive(Debug, Clone)]
pub struct BudgetSectionInput {
    pub id: &'static str,
    pub section_type: &'static str,
    pub title: &'static str,
    pub priority: u8,
    pub text: String,
    pub can_drop: bool,
    pub can_truncate: bool,
    pub reason: &'static str,
}

#[derive(Default)]
pub struct ContextBudgetAllocator {
    config: TokenCounterConfig,
}


impl ContextBudgetAllocator {
    pub fn new(config: TokenCounterConfig) -> Self {
        Self { config }
    }

    pub fn configure(&mut self, config: TokenCounterConfig) {
        self.config = config;
    }

    pub fn build(
        &self,
        sections: Vec<BudgetSectionInput>,
        tool_schema_tokens: u64,
        dialogue_messages: &[(String, String)],
    ) -> ContextBudgetAllocation {
        let max_input = max_input_tokens(&self.config, tool_schema_tokens);
        let mut sorted = sections;
        sorted.sort_by_key(|s| std::cmp::Reverse(s.priority));

        let mut used = 0u64;
        let mut dropped_message_count = 0u64;
        let mut truncated_message_count = 0u64;
        let mut budget_sections = Vec::with_capacity(sorted.len());

        for section in sorted {
            let allocated = section_allocation(section.section_type, max_input);
            let remaining = max_input.saturating_sub(used);
            let (text, truncated) = fit_text(
                &section.text,
                remaining.min(allocated),
                section.can_truncate,
            );
            let tokens = estimate_tokens(&text);
            let dropped = text.is_empty() && !section.text.trim().is_empty();
            if truncated {
                truncated_message_count += 1;
            }
            if dropped {
                dropped_message_count += 1;
            } else if !text.is_empty() {
                used += tokens;
            }
            budget_sections.push(ContextBudgetSection {
                id: section.id.to_string(),
                section_type: section.section_type.to_string(),
                title: section.title.to_string(),
                priority: section.priority,
                allocated_tokens: allocated,
                used_tokens: tokens,
                dropped,
                truncated,
                loaded: !text.is_empty(),
                rendered_text: text,
                reason: if dropped {
                    format!(
                        "{} Dropped because the context budget was exhausted.",
                        section.reason
                    )
                } else if truncated {
                    format!("{} Truncated to fit token allocation.", section.reason)
                } else {
                    format!("{} Included without trimming.", section.reason)
                },
            });
        }

        let dialogue_tokens = dialogue_messages
            .iter()
            .map(|(_, content)| estimate_tokens(content))
            .sum::<u64>();
        used = used.saturating_add(dialogue_tokens);
        let breakdown = TokenCountBreakdown {
            input_tokens: used,
            output_tokens: 0,
            total_tokens: used.saturating_add(tool_schema_tokens),
            tool_schema_tokens,
        };

        ContextBudgetAllocation {
            sections: budget_sections,
            max_input_tokens: max_input,
            tool_schema_tokens,
            used_input_tokens: used,
            remaining_input_tokens: max_input.saturating_sub(used),
            dropped_message_count,
            truncated_message_count,
            breakdown,
        }
    }
}

pub fn section_token_allocation(section_type: &str, max_input_tokens: u64) -> u64 {
    section_allocation(section_type, max_input_tokens)
}

fn section_allocation(section_type: &str, max_input_tokens: u64) -> u64 {
    let ratio = match section_type {
        "system" => 0.17,
        "project_instructions" => 0.08,
        "memory_profile" => 0.06,
        "memory" => 0.06,
        "skills" => 0.05,
        "key_facts" => 0.04,
        "delegation" => 0.04,
        "recent_dialogue" => 0.22,
        "tool_results" => 0.18,
        _ => 0.05,
    };
    ((max_input_tokens as f64) * ratio).floor().max(32.0) as u64
}

fn fit_text(text: &str, max_tokens: u64, can_truncate: bool) -> (String, bool) {
    if text.trim().is_empty() || max_tokens == 0 {
        return (String::new(), false);
    }
    let current = estimate_tokens(text);
    if current <= max_tokens {
        return (text.to_string(), false);
    }
    if !can_truncate || max_tokens < 24 {
        return (String::new(), false);
    }
    let mut ascii = 0u64;
    let mut cjk = 0u64;
    for ch in text.chars() {
        if matches!(ch, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}') {
            cjk += 1;
        } else {
            ascii += 1;
        }
    }
    let estimated = ascii.div_ceil(4) + ((cjk as f64 * 1.5).ceil() as u64);
    let chars_per_token = if estimated > 0 {
        text.chars().count() as f64 / estimated as f64
    } else {
        4.0
    };
    let max_chars = ((max_tokens as f64) * chars_per_token * 0.85).floor().max(80.0) as usize;
    let truncated: String = text.chars().take(max_chars).collect();
    (
        format!("{truncated}\n\n[Truncated by context budget allocator]"),
        true,
    )
}
