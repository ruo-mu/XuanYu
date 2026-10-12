//! Persistent channel delivery ledger (inbound/outbound) under data_dir/channels.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_STORED: usize = 2000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDeliveryRecord {
    pub id: String,
    pub channel_id: String,
    pub direction: String,
    pub external_user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub text_preview: String,
    pub status: String,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn deliveries_path(data_dir: &Path) -> PathBuf {
    data_dir.join("channels").join("deliveries.jsonl")
}

fn preview_text(text: &str) -> String {
    let trimmed = text.trim();
    let chars: String = trimmed.chars().take(240).collect();
    if trimmed.chars().count() > 240 {
        format!("{chars}…")
    } else {
        chars
    }
}

fn new_delivery_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("dlv_{nanos:x}")
}

pub struct AppendDeliveryInput<'a> {
    pub data_dir: &'a Path,
    pub channel_id: &'a str,
    pub direction: &'a str,
    pub external_user_id: &'a str,
    pub session_id: Option<&'a str>,
    pub text: &'a str,
    pub status: &'a str,
    pub error: Option<&'a str>,
}

pub fn append_delivery(input: AppendDeliveryInput<'_>) -> ChannelDeliveryRecord {
    let AppendDeliveryInput {
        data_dir,
        channel_id,
        direction,
        external_user_id,
        session_id,
        text,
        status,
        error,
    } = input;
    let record = ChannelDeliveryRecord {
        id: new_delivery_id(),
        channel_id: channel_id.to_string(),
        direction: direction.to_string(),
        external_user_id: external_user_id.to_string(),
        session_id: session_id.map(str::to_string),
        text_preview: preview_text(text),
        status: status.to_string(),
        created_at: Utc::now().timestamp_millis(),
        error: error.map(str::to_string),
    };
    if let Err(err) = append_record(data_dir, &record) {
        tracing::warn!("channel delivery append failed: {err:#}");
    }
    record
}

fn append_record(data_dir: &Path, record: &ChannelDeliveryRecord) -> anyhow::Result<()> {
    let path = deliveries_path(data_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(file, "{}", serde_json::to_string(record)?)?;
    Ok(())
}

pub fn list_deliveries(data_dir: &Path, limit: usize) -> Vec<ChannelDeliveryRecord> {
    let path = deliveries_path(data_dir);
    let Ok(file) = fs::File::open(&path) else {
        return Vec::new();
    };
    let limit = limit.clamp(1, 500);
    let mut records = Vec::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<ChannelDeliveryRecord>(line) {
            records.push(record);
        }
    }
    if records.len() > MAX_STORED {
        let keep = records.split_off(records.len() - MAX_STORED);
        let _ = rewrite_all(data_dir, &keep);
        records = keep;
    }
    let start = records.len().saturating_sub(limit);
    records[start..].iter().rev().cloned().collect()
}

fn rewrite_all(data_dir: &Path, records: &[ChannelDeliveryRecord]) -> anyhow::Result<()> {
    let path = deliveries_path(data_dir);
    let tmp = path.with_extension("jsonl.tmp");
    {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        for record in records {
            writeln!(file, "{}", serde_json::to_string(record)?)?;
        }
    }
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn deliveries_payload(data_dir: &Path, limit: usize) -> serde_json::Value {
    json!({ "deliveries": list_deliveries(data_dir, limit) })
}
