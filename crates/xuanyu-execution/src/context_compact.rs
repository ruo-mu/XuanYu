use serde::{Deserialize, Serialize};

/// Compact strategies from context manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactStrategyKind {
    Autocompact,
    Microcompact,
    Snip,
    Reactive,
}

impl CompactStrategyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Autocompact => "autocompact",
            Self::Microcompact => "microcompact",
            Self::Snip => "snip",
            Self::Reactive => "reactive",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "microcompact" => Self::Microcompact,
            "snip" => Self::Snip,
            "reactive" => Self::Reactive,
            _ => Self::Autocompact,
        }
    }

    pub fn threshold(self) -> f64 {
        match self {
            // Align closer to reference autocompact ~0.85 for the common path.
            Self::Autocompact => 0.85,
            Self::Microcompact => 0.80,
            Self::Snip => 0.92,
            Self::Reactive => 0.98,
        }
    }
}

pub const MIN_MESSAGES_FOR_COMPACT: usize = 8;
/// Preserve this many recent *user rounds* (not raw messages).
pub const PRESERVE_RECENT_USER_ROUNDS: usize = 6;
pub const PRESERVE_RECENT_COUNT: usize = 6;
pub const MAX_COMPACTION_ATTEMPTS: usize = 3;

pub fn snip_message(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        return content.to_string();
    }
    let mut out = content.chars().take(max_chars).collect::<String>();
    out.push_str("\n...[snipped]");
    out
}

/// Legacy message-count keep index (fallback when roles are unavailable).
pub fn select_compact_keep_index(message_count: usize, strategy: CompactStrategyKind) -> usize {
    let preserve = match strategy {
        CompactStrategyKind::Microcompact | CompactStrategyKind::Reactive => PRESERVE_RECENT_COUNT,
        CompactStrategyKind::Snip => message_count,
        CompactStrategyKind::Autocompact => PRESERVE_RECENT_COUNT + 2,
    };
    message_count.saturating_sub(preserve)
}

/// Keep index by user rounds: drop older messages while preserving the last N user turns
/// and everything after the N-th-last user message.
pub fn select_compact_keep_index_by_roles(
    roles: &[impl AsRef<str>],
    strategy: CompactStrategyKind,
) -> usize {
    if matches!(strategy, CompactStrategyKind::Snip) {
        return roles.len();
    }
    let preserve_rounds = match strategy {
        CompactStrategyKind::Autocompact => PRESERVE_RECENT_USER_ROUNDS + 1,
        _ => PRESERVE_RECENT_USER_ROUNDS,
    };
    let mut user_indexes = Vec::new();
    for (index, role) in roles.iter().enumerate() {
        if role.as_ref().eq_ignore_ascii_case("user") {
            user_indexes.push(index);
        }
    }
    if user_indexes.len() <= preserve_rounds {
        return 0;
    }
    
    user_indexes[user_indexes.len() - preserve_rounds]
}

pub fn should_compact(usage_ratio: f64, message_count: usize, strategy: CompactStrategyKind) -> bool {
    message_count >= MIN_MESSAGES_FOR_COMPACT && usage_ratio >= strategy.threshold()
}
