use serde_json::Value;

/// Turn-scoped memory text cache — mirrors `Agent.turnMemoryTextCache` / `turnMemoryQuery`.
#[derive(Debug, Default, Clone)]
pub struct TurnMemoryCache {
    memory_text: Option<String>,
    memory_query: Option<String>,
}

impl TurnMemoryCache {
    pub fn invalidate(&mut self) {
        self.memory_text = None;
        self.memory_query = None;
    }

    pub fn should_invalidate_for_sections(sections: &[String]) -> bool {
        sections.iter().any(|section| {
            matches!(
                section.as_str(),
                "memory" | "memory_profile" | "project_instructions"
            )
        })
    }

    pub fn refresh_after_context_sections_loaded(&mut self, sections: &[String]) {
        if Self::should_invalidate_for_sections(sections) {
            self.invalidate();
        }
    }

    pub fn get_or_compute<F>(&mut self, query: &str, compute: F) -> Option<String>
    where
        F: FnOnce(&str) -> Option<String>,
    {
        if self
            .memory_query
            .as_deref()
            .is_some_and(|cached| cached == query)
        {
            return self.memory_text.clone();
        }
        let text = compute(query)?;
        self.memory_query = Some(query.to_string());
        self.memory_text = Some(text.clone());
        Some(text)
    }

    pub fn snapshot(&self) -> Value {
        serde_json::json!({
            "hasCache": self.memory_text.is_some(),
            "query": self.memory_query,
        })
    }
}
