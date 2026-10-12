use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, path::PathBuf};
use xuanyu_storage::project_data_root;

pub const MAX_GOAL_OBJECTIVE_CHARS: usize = 4000;
pub const DEFAULT_MAX_CONTINUATIONS: u32 = 64;
pub const MAX_LEDGER_ENTRIES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionGoalStatus {
    Active,
    Paused,
    Complete,
    BudgetLimited,
    Cleared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionGoalLedgerKind {
    Created,
    Progress,
    Checkpoint,
    Continuation,
    Paused,
    Resumed,
    Budget,
    Completed,
    Cleared,
    Edited,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionGoalLedgerEntry {
    pub at: i64,
    pub kind: SessionGoalLedgerKind,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_refs: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionGoal {
    pub id: String,
    pub session_id: String,
    pub objective: String,
    pub status: SessionGoalStatus,
    #[serde(default)]
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub verification_hints: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
    #[serde(default)]
    pub tokens_used: u64,
    pub started_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    #[serde(default)]
    pub ledger: Vec<SessionGoalLedgerEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_continuation_at: Option<i64>,
    #[serde(default)]
    pub suppress_next_continuation: bool,
    #[serde(default)]
    pub continuation_count: u32,
    #[serde(default = "default_max_continuations")]
    pub max_continuations: u32,
    #[serde(default)]
    pub linked_run_ids: Vec<String>,
}

fn default_max_continuations() -> u32 {
    DEFAULT_MAX_CONTINUATIONS
}

#[derive(Debug, Clone)]
pub struct SessionGoalStore {
    project_root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalActionResult {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continue_message: Option<String>,
}

impl SessionGoalStore {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
        }
    }

    pub fn goal_path(&self, session_id: &str) -> PathBuf {
        project_data_root(&self.project_root)
            .join("goals")
            .join(format!("{session_id}.json"))
    }

    pub fn load(&self, session_id: &str) -> Option<SessionGoal> {
        let path = self.goal_path(session_id);
        let text = fs::read_to_string(&path).ok()?;
        let raw = serde_json::from_str::<SessionGoal>(&text).ok()?;
        if raw.id.trim().is_empty() || raw.objective.trim().is_empty() {
            return None;
        }
        Some(normalize_goal(raw, session_id))
    }

    pub fn save(&self, goal: &SessionGoal) -> anyhow::Result<()> {
        let path = self.goal_path(&goal.session_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        let text = serde_json::to_string_pretty(goal)?;
        fs::write(&tmp, format!("{text}\n"))?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn delete(&self, session_id: &str) {
        let _ = fs::remove_file(self.goal_path(session_id));
    }

    pub fn wire_snapshot(&self, session_id: &str) -> Value {
        let goal = self.load(session_id);
        json!({
            "sessionId": session_id,
            "goal": goal.and_then(goal_wire_payload)
        })
    }

    pub fn apply_action(
        &self,
        session_id: &str,
        action: &str,
        objective: Option<&str>,
        token_budget: Option<u64>,
    ) -> GoalActionResult {
        match action {
            "pause" => match self.pause(session_id) {
                Ok(_) => GoalActionResult {
                    ok: true,
                    error: None,
                    continue_message: None,
                },
                Err(error) => GoalActionResult {
                    ok: false,
                    error: Some(error),
                    continue_message: None,
                },
            },
            "resume" => match self.resume(session_id) {
                Ok(goal) => GoalActionResult {
                    ok: true,
                    error: None,
                    continue_message: Some(goal.objective.clone()),
                },
                Err(error) => GoalActionResult {
                    ok: false,
                    error: Some(error),
                    continue_message: None,
                },
            },
            "clear" => {
                let _ = self.clear(session_id);
                GoalActionResult {
                    ok: true,
                    error: None,
                    continue_message: None,
                }
            }
            "edit" => {
                let Some(text) = objective.map(str::trim).filter(|value| !value.is_empty()) else {
                    return GoalActionResult {
                        ok: false,
                        error: Some("objective required".to_string()),
                        continue_message: None,
                    };
                };
                match self.edit(session_id, text) {
                    Ok(_) => GoalActionResult {
                        ok: true,
                        error: None,
                        continue_message: None,
                    },
                    Err(error) => GoalActionResult {
                        ok: false,
                        error: Some(error),
                        continue_message: None,
                    },
                }
            }
            "budget" => {
                let Some(budget) = token_budget.filter(|value| *value > 0) else {
                    return GoalActionResult {
                        ok: false,
                        error: Some("tokenBudget required".to_string()),
                        continue_message: None,
                    };
                };
                match self.set_budget(session_id, budget) {
                    Ok(_) => GoalActionResult {
                        ok: true,
                        error: None,
                        continue_message: None,
                    },
                    Err(error) => GoalActionResult {
                        ok: false,
                        error: Some(error),
                        continue_message: None,
                    },
                }
            }
            _ => GoalActionResult {
                ok: false,
                error: Some("Unknown action".to_string()),
                continue_message: None,
            },
        }
    }

    fn pause(&self, session_id: &str) -> Result<SessionGoal, String> {
        self.set_status(
            session_id,
            SessionGoalStatus::Paused,
            SessionGoalLedgerKind::Paused,
            "Goal paused by user",
        )
    }

    fn resume(&self, session_id: &str) -> Result<SessionGoal, String> {
        let Some(goal) = self.load(session_id) else {
            return Err("No goal to resume.".to_string());
        };
        if goal.status == SessionGoalStatus::Cleared {
            return Err("No goal to resume.".to_string());
        }
        if goal.status == SessionGoalStatus::Complete {
            return Err("Goal already complete.".to_string());
        }
        if goal.status == SessionGoalStatus::Active {
            return Ok(goal);
        }
        let mut next = goal;
        next.status = SessionGoalStatus::Active;
        next.suppress_next_continuation = false;
        next.updated_at = now_ms();
        next = append_ledger(
            next,
            SessionGoalLedgerKind::Resumed,
            "Goal resumed by user",
            None,
        );
        self.save(&next).map_err(|err| err.to_string())?;
        Ok(next)
    }

    fn clear(&self, session_id: &str) -> Option<SessionGoal> {
        let Some(goal) = self.load(session_id) else {
            self.delete(session_id);
            return None;
        };
        if goal.status == SessionGoalStatus::Cleared {
            self.delete(session_id);
            return None;
        }
        let mut next = goal;
        next.status = SessionGoalStatus::Cleared;
        next.updated_at = now_ms();
        next = append_ledger(
            next,
            SessionGoalLedgerKind::Cleared,
            "Goal cleared",
            None,
        );
        let _ = self.save(&next);
        Some(next)
    }

    fn edit(&self, session_id: &str, objective: &str) -> Result<SessionGoal, String> {
        let Some(goal) = self.load(session_id) else {
            return Err("No goal to edit.".to_string());
        };
        if goal.status == SessionGoalStatus::Cleared {
            return Err("No goal to edit.".to_string());
        }
        if goal.status == SessionGoalStatus::Complete {
            return Err("Completed goals cannot be edited; create a new one.".to_string());
        }
        let text = objective.trim();
        if text.is_empty() {
            return Err("Edited objective must be non-empty.".to_string());
        }
        let mut next = goal;
        next.objective = text.chars().take(MAX_GOAL_OBJECTIVE_CHARS).collect();
        next.updated_at = now_ms();
        next = append_ledger(
            next,
            SessionGoalLedgerKind::Edited,
            format!("Objective updated: {}", text.chars().take(240).collect::<String>()),
            None,
        );
        if next.status == SessionGoalStatus::BudgetLimited {
            next.status = SessionGoalStatus::Paused;
        }
        self.save(&next).map_err(|err| err.to_string())?;
        Ok(next)
    }

    fn set_budget(&self, session_id: &str, token_budget: u64) -> Result<SessionGoal, String> {
        let Some(goal) = self.load(session_id) else {
            return Err("No goal for budget.".to_string());
        };
        if goal.status == SessionGoalStatus::Cleared {
            return Err("No goal for budget.".to_string());
        }
        let mut next = goal;
        next.token_budget = Some(token_budget);
        next.updated_at = now_ms();
        next = append_ledger(
            next,
            SessionGoalLedgerKind::Budget,
            format!("Token budget set to {token_budget}"),
            None,
        );
        if next.tokens_used >= token_budget {
            next.status = SessionGoalStatus::BudgetLimited;
            next = append_ledger(
                next,
                SessionGoalLedgerKind::Budget,
                "Already over budget; goal limited".to_string(),
                None,
            );
        }
        self.save(&next).map_err(|err| err.to_string())?;
        Ok(next)
    }

    fn set_status(
        &self,
        session_id: &str,
        status: SessionGoalStatus,
        ledger_kind: SessionGoalLedgerKind,
        summary: &str,
    ) -> Result<SessionGoal, String> {
        let Some(goal) = self.load(session_id) else {
            return Err("No goal.".to_string());
        };
        if goal.status == SessionGoalStatus::Cleared {
            return Err("No goal.".to_string());
        }
        if goal.status == SessionGoalStatus::Complete {
            return Err("Goal already complete.".to_string());
        }
        let mut next = goal;
        next.status = status;
        next.updated_at = now_ms();
        next.suppress_next_continuation = status != SessionGoalStatus::Active;
        next = append_ledger(next, ledger_kind, summary, None);
        self.save(&next).map_err(|err| err.to_string())?;
        Ok(next)
    }
}

fn normalize_goal(mut raw: SessionGoal, session_id: &str) -> SessionGoal {
    raw.session_id = session_id.to_string();
    if raw.success_criteria.is_empty() {
        raw.success_criteria = Vec::new();
    }
    if raw.verification_hints.is_empty() {
        raw.verification_hints = Vec::new();
    }
    if raw.constraints.is_empty() {
        raw.constraints = Vec::new();
    }
    if raw.ledger.is_empty() {
        raw.ledger = Vec::new();
    }
    if raw.linked_run_ids.is_empty() {
        raw.linked_run_ids = Vec::new();
    }
    if raw.max_continuations == 0 {
        raw.max_continuations = DEFAULT_MAX_CONTINUATIONS;
    }
    raw
}

fn append_ledger(
    mut goal: SessionGoal,
    kind: SessionGoalLedgerKind,
    summary: impl Into<String>,
    evidence_refs: Option<Vec<String>>,
) -> SessionGoal {
    let at = now_ms();
    goal.ledger.push(SessionGoalLedgerEntry {
        at,
        kind,
        summary: summary.into().chars().take(500).collect(),
        evidence_refs: evidence_refs.map(|items| items.into_iter().take(12).collect()),
    });
    if goal.ledger.len() > MAX_LEDGER_ENTRIES {
        let drain = goal.ledger.len() - MAX_LEDGER_ENTRIES;
        goal.ledger.drain(0..drain);
    }
    goal.updated_at = at;
    goal
}

pub fn create_session_goal(session_id: &str, objective: &str) -> SessionGoal {
    let now = now_ms();
    let objective = objective.trim().chars().take(MAX_GOAL_OBJECTIVE_CHARS).collect::<String>();
    SessionGoal {
        id: format!("sg_{now}_{}", (now ^ (now >> 16)) & 0x000f_ffff),
        session_id: session_id.to_string(),
        objective: objective.clone(),
        status: SessionGoalStatus::Active,
        success_criteria: Vec::new(),
        verification_hints: Vec::new(),
        constraints: Vec::new(),
        token_budget: None,
        tokens_used: 0,
        started_at: now,
        updated_at: now,
        completed_at: None,
        ledger: vec![SessionGoalLedgerEntry {
            at: now,
            kind: SessionGoalLedgerKind::Created,
            summary: format!("Goal created: {}", objective.chars().take(240).collect::<String>()),
            evidence_refs: None,
        }],
        last_continuation_at: None,
        suppress_next_continuation: false,
        continuation_count: 0,
        max_continuations: DEFAULT_MAX_CONTINUATIONS,
        linked_run_ids: Vec::new(),
    }
}

fn goal_wire_payload(goal: SessionGoal) -> Option<Value> {
    if matches!(
        goal.status,
        SessionGoalStatus::Cleared | SessionGoalStatus::Complete
    ) {
        return None;
    }
    let status = match goal.status {
        SessionGoalStatus::Active => "active",
        SessionGoalStatus::Paused => "paused",
        SessionGoalStatus::BudgetLimited => "budget_limited",
        SessionGoalStatus::Complete => "complete",
        SessionGoalStatus::Cleared => "cleared",
    };
    Some(json!({
        "id": goal.id,
        "objective": goal.objective,
        "status": status,
        "successCriteria": goal.success_criteria,
        "tokenBudget": goal.token_budget,
        "tokensUsed": goal.tokens_used,
        "startedAt": goal.started_at,
        "updatedAt": goal.updated_at,
        "continuationCount": goal.continuation_count,
        "maxContinuations": goal.max_continuations,
        "recentLedger": goal.ledger.into_iter().rev().take(5).rev().map(|entry| json!({
            "at": entry.at,
            "kind": ledger_kind_wire(&entry.kind),
            "summary": entry.summary,
        })).collect::<Vec<_>>(),
    }))
}

fn ledger_kind_wire(kind: &SessionGoalLedgerKind) -> &'static str {
    match kind {
        SessionGoalLedgerKind::Created => "created",
        SessionGoalLedgerKind::Progress => "progress",
        SessionGoalLedgerKind::Checkpoint => "checkpoint",
        SessionGoalLedgerKind::Continuation => "continuation",
        SessionGoalLedgerKind::Paused => "paused",
        SessionGoalLedgerKind::Resumed => "resumed",
        SessionGoalLedgerKind::Budget => "budget",
        SessionGoalLedgerKind::Completed => "completed",
        SessionGoalLedgerKind::Cleared => "cleared",
        SessionGoalLedgerKind::Edited => "edited",
    }
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}
