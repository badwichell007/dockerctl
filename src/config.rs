use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::AppResult;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub groups: GroupConfig,
    #[serde(default)]
    pub tui: TuiConfig,
    #[serde(default)]
    pub safety: SafetyConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            groups: GroupConfig::default(),
            tui: TuiConfig::default(),
            safety: SafetyConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupConfig {
    #[serde(default)]
    pub exact: BTreeMap<String, String>,
    #[serde(default)]
    pub prefix: Vec<(String, String)>,
    #[serde(default)]
    pub image_prefix: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiConfig {
    pub refresh_ms: u64,
    pub log_tail: usize,
    pub default_filter: String,
    pub theme: String,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            refresh_ms: 2_000,
            log_tail: 200,
            default_filter: String::new(),
            theme: "industrial".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetyConfig {
    pub typed_confirmation: bool,
    pub allow_yes_for_purge: bool,
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            typed_confirmation: true,
            allow_yes_for_purge: false,
        }
    }
}

pub fn config_path() -> Option<PathBuf> {
    if let Some(base) = env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(base).join("dockerctl/config.toml"));
    }
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/dockerctl/config.toml"))
}

pub fn state_dir_path() -> Option<PathBuf> {
    if let Some(base) = env::var_os("XDG_STATE_HOME") {
        return Some(PathBuf::from(base).join("dockerctl"));
    }
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state/dockerctl"))
}

pub fn audit_log_path() -> Option<PathBuf> {
    state_dir_path().map(|path| path.join("audit.log"))
}

pub fn timeline_log_path() -> Option<PathBuf> {
    state_dir_path().map(|path| path.join("timeline.jsonl"))
}

pub fn load_config() -> AppConfig {
    let Some(path) = config_path() else {
        return AppConfig::default();
    };
    let Ok(content) = fs::read_to_string(path) else {
        return AppConfig::default();
    };
    toml::from_str::<AppConfig>(&content).unwrap_or_else(|_| AppConfig {
        groups: parse_group_config(&content),
        ..AppConfig::default()
    })
}

pub fn write_default_config(path: &PathBuf) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, toml::to_string_pretty(&AppConfig::default())?)?;
    Ok(())
}

pub fn parse_group_config(content: &str) -> GroupConfig {
    enum Section {
        None,
        Exact,
        Prefix,
        ImagePrefix,
    }

    let mut config = GroupConfig::default();
    let mut section = Section::None;
    for raw_line in content.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let section_name = line
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .to_ascii_lowercase();
            section = match section_name.as_str() {
                "groups.exact" | "group_exact" | "standalone_group_exact" => Section::Exact,
                "groups.prefix" | "group_prefix" | "standalone_group_prefix" => Section::Prefix,
                "groups.image_prefix" | "group_image_prefix" | "standalone_group_image_prefix" => {
                    Section::ImagePrefix
                }
                _ => Section::None,
            };
            continue;
        }

        let Some((left, right)) = line.split_once('=') else {
            continue;
        };
        let key = parse_config_atom(left);
        let value = parse_config_atom(right);
        if key.is_empty() || value.is_empty() {
            continue;
        }

        match section {
            Section::Exact => {
                config.exact.insert(key, value);
            }
            Section::Prefix => config.prefix.push((key, value)),
            Section::ImagePrefix => config.image_prefix.push((key, value)),
            Section::None => {}
        }
    }
    config
}

pub fn parse_config_atom(raw: &str) -> String {
    let mut value = raw.trim().to_string();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value = value[1..value.len() - 1].to_string();
    }
    value.trim().to_string()
}
