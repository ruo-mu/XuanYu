use serde::{Deserialize, Serialize};

use crate::SubAgentTaskResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeConflictSeverity {
    Blocking,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeConflictType {
    File,
    Logic,
    Validation,
    Style,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentConflict {
    #[serde(rename = "type")]
    pub conflict_type: MergeConflictType,
    pub severity: MergeConflictSeverity,
    pub message: String,
    pub files: Vec<String>,
    pub agent_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentMergeAssessment {
    pub can_merge: bool,
    pub blocking_count: usize,
    pub warning_count: usize,
    pub conflicts: Vec<SubAgentConflict>,
    pub summary: String,
}

/// Mirrors `SubAgentManager.assessMerge` (manager.ts:716).
pub fn assess_merge(results: &[SubAgentTaskResult]) -> SubAgentMergeAssessment {
    let mut conflicts = Vec::new();
    let mut by_file: std::collections::BTreeMap<String, Vec<&SubAgentTaskResult>> =
        std::collections::BTreeMap::new();
    for result in results {
        let files = if !result.modified_files.is_empty() {
            result.modified_files.clone()
        } else {
            result.write_scope.clone()
        };
        for file in normalize_file_list(&files) {
            by_file.entry(file).or_default().push(result);
        }
    }
    for (file, owners) in by_file {
        if owners.len() > 1 {
            let agent_ids = owners
                .iter()
                .map(|owner| owner.agent_id.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            conflicts.push(SubAgentConflict {
                conflict_type: MergeConflictType::File,
                severity: MergeConflictSeverity::Blocking,
                message: format!(
                    "Multiple write-capable SubAgents touched {file}. Merge must be serialized and reviewed."
                ),
                files: vec![file],
                agent_ids,
            });
        }
    }
    for result in results {
        for blocker in &result.blockers {
            conflicts.push(SubAgentConflict {
                conflict_type: MergeConflictType::Logic,
                severity: if result.status == "failed" {
                    MergeConflictSeverity::Blocking
                } else {
                    MergeConflictSeverity::Warning
                },
                message: format!("{} reported blockers: {blocker}", result.agent_name),
                files: normalize_file_list(&result.modified_files),
                agent_ids: vec![result.agent_id.clone()],
            });
        }
        for risk in &result.risks {
            let lower = risk.to_lowercase();
            if lower.contains("fail")
                || lower.contains("失败")
                || lower.contains("错误")
                || lower.contains("error")
                || lower.contains("验证")
            {
                conflicts.push(SubAgentConflict {
                    conflict_type: MergeConflictType::Validation,
                    severity: MergeConflictSeverity::Warning,
                    message: format!("{} reported verification risk: {risk}", result.agent_name),
                    files: normalize_file_list(&result.modified_files),
                    agent_ids: vec![result.agent_id.clone()],
                });
            } else if lower.contains("style")
                || lower.contains("format")
                || lower.contains("lint")
                || lower.contains("风格")
                || lower.contains("格式")
            {
                conflicts.push(SubAgentConflict {
                    conflict_type: MergeConflictType::Style,
                    severity: MergeConflictSeverity::Warning,
                    message: format!("{} reported style risk: {risk}", result.agent_name),
                    files: normalize_file_list(&result.modified_files),
                    agent_ids: vec![result.agent_id.clone()],
                });
            }
        }
    }
    build_merge_assessment(conflicts, results)
}

fn normalize_file_list(files: &[String]) -> Vec<String> {
    files
        .iter()
        .map(|file| file.replace('\\', "/"))
        .filter(|file| !file.trim().is_empty())
        .collect()
}

fn build_merge_assessment(
    conflicts: Vec<SubAgentConflict>,
    results: &[SubAgentTaskResult],
) -> SubAgentMergeAssessment {
    let blocking_count = conflicts
        .iter()
        .filter(|item| item.severity == MergeConflictSeverity::Blocking)
        .count();
    let warning_count = conflicts.len() - blocking_count;
    let can_merge = blocking_count == 0;
    let summary = if can_merge {
        format!(
            "Merge assessment passed for {} sub-agent result(s) with {warning_count} warning(s).",
            results.len()
        )
    } else {
        format!(
            "Merge blocked: {blocking_count} blocking conflict(s), {warning_count} warning(s)."
        )
    };
    SubAgentMergeAssessment {
        can_merge,
        blocking_count,
        warning_count,
        conflicts,
        summary,
    }
}
