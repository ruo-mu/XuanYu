#[derive(Debug, Clone)]
pub struct CommandValidationResult {
    pub safe: bool,
    pub reason: Option<String>,
}

/// Mirrors `governance/guards/commandValidator.ts` `CommandValidator`.
pub struct CommandValidator;

impl CommandValidator {
    pub fn validate(command: &str) -> CommandValidationResult {
        let trimmed = command.trim();
        const DANGEROUS: &[&str] = &[
            "rm -rf /",
            "dd if=/dev/zero",
            "mkfs",
            "format",
            ":(){ :|:& };:",
        ];
        if DANGEROUS.contains(&trimmed) {
            return CommandValidationResult {
                safe: false,
                reason: Some("Dangerous command blocked".to_string()),
            };
        }
        let patterns = [
            r"\n",
            r"\r",
            r"rm\s+-rf\s+/",
            r"sudo\s+rm",
            r"chmod\s+777",
            r"eval\s+",
            r">\s*/dev/sd",
            r"\bdd\s+.*of=/dev/",
            r"\b(shutdown|reboot|halt|poweroff)\b",
            r"\b(format|diskpart)\b",
        ];
        for pattern in patterns {
            if regex::Regex::new(pattern)
                .map(|re| re.is_match(command))
                .unwrap_or(false)
            {
                return CommandValidationResult {
                    safe: false,
                    reason: Some("Dangerous pattern detected".to_string()),
                };
            }
        }
        CommandValidationResult {
            safe: true,
            reason: None,
        }
    }

    pub fn is_dangerous(command: &str) -> CommandValidationResult {
        Self::validate(command)
    }
}
