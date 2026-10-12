use chrono::Utc;
use std::collections::BTreeMap;
use tokio::sync::RwLock;
use uuid::Uuid;
use xuanyu_protocol::{SessionParticipant, SESSION_DEFAULT_PARTICIPANT_ID};

#[derive(Default)]
pub struct SessionParticipantRegistry {
    by_session: RwLock<BTreeMap<String, BTreeMap<String, SessionParticipant>>>,
}

impl SessionParticipantRegistry {
    pub async fn ensure_default(&self, session_id: &str) {
        let mut sessions = self.by_session.write().await;
        let participants = sessions.entry(session_id.to_string()).or_default();
        if participants.is_empty() {
            let now = Utc::now();
            participants.insert(
                SESSION_DEFAULT_PARTICIPANT_ID.to_string(),
                SessionParticipant {
                    id: SESSION_DEFAULT_PARTICIPANT_ID.to_string(),
                    role: "user".to_string(),
                    display_name: Some("Local User".to_string()),
                    external_id: None,
                    last_seen_at: now,
                },
            );
        }
    }

    pub async fn join(
        &self,
        session_id: &str,
        role: impl Into<String>,
        display_name: Option<String>,
        external_id: Option<String>,
    ) -> SessionParticipant {
        let participant_id = external_id
            .as_ref()
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("participant_{}", Uuid::new_v4().simple()));
        let now = Utc::now();
        let participant = SessionParticipant {
            id: participant_id.clone(),
            role: role.into(),
            display_name,
            external_id,
            last_seen_at: now,
        };
        self.by_session
            .write()
            .await
            .entry(session_id.to_string())
            .or_default()
            .insert(participant_id, participant.clone());
        participant
    }

    pub async fn touch(&self, session_id: &str, participant_id: &str) -> bool {
        let mut sessions = self.by_session.write().await;
        let Some(participants) = sessions.get_mut(session_id) else {
            return false;
        };
        let Some(participant) = participants.get_mut(participant_id) else {
            return false;
        };
        participant.last_seen_at = Utc::now();
        true
    }

    pub async fn list(&self, session_id: &str) -> Vec<SessionParticipant> {
        self.by_session
            .read()
            .await
            .get(session_id)
            .map(|participants| participants.values().cloned().collect())
            .unwrap_or_default()
    }

    pub async fn find_by_external_id(
        &self,
        session_id: &str,
        external_id: &str,
    ) -> Option<SessionParticipant> {
        self.by_session
            .read()
            .await
            .get(session_id)?
            .values()
            .find(|participant| {
                participant
                    .external_id
                    .as_deref()
                    .is_some_and(|value| value == external_id)
            })
            .cloned()
    }
}

