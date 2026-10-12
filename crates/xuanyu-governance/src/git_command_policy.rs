//! Git 命令分级：结构化 git_* 与 exec 内 git 共用。
//! destructive 永远拒绝（force push、hard reset、clean -fd、rebase、filter-* 等）。

use std::collections::HashSet;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitCommandClass {
    Read,
    Mutate,
    Worktree,
    Remote,
    Destructive,
    Unknown,
}

fn destructive_subs() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        HashSet::from([
            "rebase",
            "reset",
            "clean",
            "filter-branch",
            "filter-repo",
            "replace",
        ])
    })
}

fn mutate_subs() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        HashSet::from([
            "add",
            "commit",
            "rm",
            "mv",
            "stage",
            "unstage",
            "update-index",
            "cherry-pick",
            "revert",
            "merge",
            "gc",
        ])
    })
}

fn worktree_subs() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        HashSet::from([
            "checkout",
            "switch",
            "restore",
            "sparse-checkout",
            "worktree",
        ])
    })
}

fn remote_subs() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| HashSet::from(["fetch", "pull", "push", "clone", "submodule"]))
}

fn read_subs() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        HashSet::from([
            "status",
            "diff",
            "log",
            "show",
            "blame",
            "rev-parse",
            "rev-list",
            "ls-files",
            "ls-tree",
            "ls-remote",
            "describe",
            "cat-file",
            "shortlog",
            "name-rev",
            "whatchanged",
            "version",
            "help",
            "config",
            "remote",
            "tag",
            "stash",
            "branch",
            "diff-index",
            "diff-tree",
            "merge-base",
            "symbolic-ref",
            "for-each-ref",
            "reflog",
        ])
    })
}

fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        if ch == '"' || ch == '\'' {
            let quote = ch;
            let mut token = String::new();
            for next in chars.by_ref() {
                if next == quote {
                    break;
                }
                token.push(next);
            }
            out.push(token);
            continue;
        }
        let mut token = String::from(ch);
        while let Some(next) = chars.peek().copied() {
            if next.is_whitespace() || next == '"' || next == '\'' {
                break;
            }
            token.push(next);
            chars.next();
        }
        out.push(token);
    }
    out
}

/** 从一段 shell 提取 git 子命令（忽略 sudo/env 与 -C/-c 等全局选项）。 */
pub fn extract_git_subcommand(segment: &str) -> Option<String> {
    let mut cleaned = segment.trim().to_string();
    let prefixes = [
        r"(?i)^\s*(?:sudo|env|command|builtin|time|nice|nohup)\s+",
        r#"(?i)^\s*[A-Za-z_][\w]*=(?:"[^"]*"|'[^']*'|[^\s]+)\s+"#,
    ];
    for _ in 0..4 {
        let before = cleaned.clone();
        for pattern in prefixes {
            if let Ok(re) = regex::Regex::new(pattern) {
                cleaned = re.replace(&cleaned, "").into_owned();
            }
        }
        if cleaned == before {
            break;
        }
    }
    let cleaned = cleaned.trim();
    let bin_re = regex::Regex::new(r#"(?i)^(?:"[^"]*"|'[^']*'|\S*[/\\])?git(?:\.exe)?\b"#).ok()?;
    if !bin_re.is_match(cleaned) {
        return None;
    }
    let without_bin = bin_re.replace(cleaned, "").trim().to_string();
    let tokens = tokenize(&without_bin);
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i].as_str();
        if matches!(t, "-C" | "-c" | "--git-dir" | "--work-tree") {
            i += 2;
            continue;
        }
        if t.starts_with('-') {
            i += 1;
            continue;
        }
        return Some(t.to_ascii_lowercase());
    }
    None
}

/** 单段 git 分级。 */
pub fn classify_git_segment(segment: &str) -> Option<GitCommandClass> {
    let sub = extract_git_subcommand(segment)?;
    let lower = segment.to_ascii_lowercase();
    let padded = format!(" {lower}");

    if (padded.contains(" --force")
        || padded.contains(" --force-with-lease")
        || padded.contains(" --force-if-includes")
        || regex::Regex::new(r"(?:^|\s)-f(?:\s|$)")
            .ok()
            .map(|re| re.is_match(&padded))
            .unwrap_or(false))
        && matches!(
            sub.as_str(),
            "push" | "checkout" | "clean" | "branch" | "tag"
        )
    {
        return Some(GitCommandClass::Destructive);
    }
    if padded.contains(" --hard") && sub == "reset" {
        return Some(GitCommandClass::Destructive);
    }
    if sub == "clean"
        && (padded.contains(" --force")
            || regex::Regex::new(r"(?:\s-([a-z]*f|\w*f\w*)\b)")
                .ok()
                .map(|re| re.is_match(&lower))
                .unwrap_or(false))
    {
        return Some(GitCommandClass::Destructive);
    }
    if lower.contains("filter-branch") || lower.contains("filter-repo") {
        return Some(GitCommandClass::Destructive);
    }
    if destructive_subs().contains(sub.as_str()) {
        return Some(GitCommandClass::Destructive);
    }
    if remote_subs().contains(sub.as_str()) {
        return Some(GitCommandClass::Remote);
    }
    if worktree_subs().contains(sub.as_str()) {
        return Some(GitCommandClass::Worktree);
    }
    if mutate_subs().contains(sub.as_str()) {
        return Some(GitCommandClass::Mutate);
    }
    if sub == "branch" {
        if regex::Regex::new(r"(?:^|\s)(-[a-zA-Z]*[dDmM]|--delete|--move|--force)(?:\s|$)")
            .ok()
            .map(|re| re.is_match(segment))
            .unwrap_or(false)
        {
            return Some(GitCommandClass::Destructive);
        }
        let tokens = tokenize(segment);
        let idx = tokens
            .iter()
            .position(|t| t.eq_ignore_ascii_case("branch"));
        let after = idx
            .map(|i| {
                tokens[i + 1..]
                    .iter()
                    .filter(|t| !t.starts_with('-'))
                    .count()
            })
            .unwrap_or(0);
        return Some(if after > 0 {
            GitCommandClass::Mutate
        } else {
            GitCommandClass::Read
        });
    }
    if sub == "tag" {
        if regex::Regex::new(r"(?:^|\s)(-[a-zA-Z]*[dD]|--delete|--force)(?:\s|$)")
            .ok()
            .map(|re| re.is_match(segment))
            .unwrap_or(false)
        {
            return Some(GitCommandClass::Destructive);
        }
        let tokens = tokenize(segment);
        let idx = tokens.iter().position(|t| t.eq_ignore_ascii_case("tag"));
        let after = idx
            .map(|i| {
                tokens[i + 1..]
                    .iter()
                    .filter(|t| !t.starts_with('-'))
                    .count()
            })
            .unwrap_or(0);
        return Some(if after > 0 {
            GitCommandClass::Mutate
        } else {
            GitCommandClass::Read
        });
    }
    if sub == "stash" {
        if regex::Regex::new(r"(?i)\b(drop|clear|pop|push|save|apply|branch)\b")
            .ok()
            .map(|re| re.is_match(segment))
            .unwrap_or(false)
        {
            return Some(GitCommandClass::Mutate);
        }
        return Some(GitCommandClass::Read);
    }
    if sub == "remote" {
        if regex::Regex::new(r"(?i)\b(add|remove|rm|rename|set-url|prune)\b")
            .ok()
            .map(|re| re.is_match(segment))
            .unwrap_or(false)
        {
            return Some(GitCommandClass::Mutate);
        }
        return Some(GitCommandClass::Read);
    }
    if sub == "config" {
        if regex::Regex::new(r"(?i)(?:^|\s)--global\b|(?:^|\s)--system\b")
            .ok()
            .map(|re| re.is_match(segment))
            .unwrap_or(false)
        {
            return Some(GitCommandClass::Destructive);
        }
        if regex::Regex::new(r"(?i)(?:^|\s)--unset\b|(?:^|\s)--replace-all\b|(?:^|\s)-[a-zA-Z]*e\b")
            .ok()
            .map(|re| re.is_match(segment))
            .unwrap_or(false)
        {
            return Some(GitCommandClass::Mutate);
        }
        let tokens = tokenize(segment);
        let idx = tokens
            .iter()
            .position(|t| t.eq_ignore_ascii_case("config"));
        let after = idx
            .map(|i| {
                tokens[i + 1..]
                    .iter()
                    .filter(|t| !t.starts_with('-'))
                    .count()
            })
            .unwrap_or(0);
        return Some(if after >= 2 {
            GitCommandClass::Mutate
        } else {
            GitCommandClass::Read
        });
    }
    if read_subs().contains(sub.as_str()) {
        return Some(GitCommandClass::Read);
    }
    Some(GitCommandClass::Unknown)
}

/** 管道/链式命令任一段 destructive 即整体 destructive。 */
pub fn command_has_destructive_git(command: &str) -> bool {
    for segment in split_shell_segments(command) {
        if classify_git_segment(&segment) == Some(GitCommandClass::Destructive) {
            return true;
        }
    }
    false
}

fn split_shell_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' && !in_double {
            in_single = !in_single;
            current.push(ch);
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            current.push(ch);
            continue;
        }
        if !in_single && !in_double
            && (ch == '|' || ch == ';' || ch == '&') {
                if ch == '&' && chars.peek() == Some(&'&') {
                    chars.next();
                }
                if ch == '|' && chars.peek() == Some(&'|') {
                    chars.next();
                }
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    segments.push(trimmed.to_string());
                }
                current.clear();
                continue;
            }
        current.push(ch);
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        segments.push(trimmed.to_string());
    }
    segments
}
