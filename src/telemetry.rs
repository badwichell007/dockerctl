use std::fs::{self, OpenOptions};
use std::io::Write;

use serde::{Deserialize, Serialize};

use crate::AppResult;
use crate::config::timeline_log_path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub ts: u128,
    pub source: String,
    pub action: String,
    pub actor: String,
    pub message: String,
}

pub fn write_timeline(event: &TimelineEvent) -> AppResult<()> {
    let Some(path) = timeline_log_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(event)?)?;
    Ok(())
}
