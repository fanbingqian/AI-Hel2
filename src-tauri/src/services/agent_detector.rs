use std::path::PathBuf;
use std::process::Command;

use super::agents::{AgentConfig, AgentConnectionConfig};

/// Three-layer detection for locally installed AI agents.
pub struct AgentDetector;

impl AgentDetector {
    pub fn new() -> Self {
        Self
    }

    /// Run all detection layers and return discovered agents.
    pub async fn scan_all(&self) -> Vec<AgentConfig> {
        let mut results = Vec::new();

        // Layer 1: known install paths (1ms) — platform-specific defaults
        results.extend(self.layer1_known_paths());

        // Layer 2: where/which command (100ms)
        results.extend(self.layer2_which());

        // Layer 3: login shell probe (4s timeout) — scan for OpenClaw config
        results.extend(self.layer3_shell_probe());

        // Deduplicate by id, keeping first occurrence
        let mut seen = std::collections::HashSet::new();
        results.retain(|a| seen.insert(a.id.clone()));

        results
    }

    fn layer1_known_paths(&self) -> Vec<AgentConfig> {
        let mut agents = Vec::new();

        #[cfg(target_os = "windows")]
        let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:".into());
        #[cfg(not(target_os = "windows"))]
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());

        // Claude Code
        #[cfg(target_os = "windows")]
        {
            let cc_path = PathBuf::from(&home).join("AppData").join("Roaming").join("npm").join("claude.cmd");
            if cc_path.exists() {
                agents.push(self.make_claude_code_agent(Some(cc_path)));
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            for p in &[
                PathBuf::from(&home).join(".local").join("bin").join("claude"),
                PathBuf::from("/usr/local/bin/claude"),
            ] {
                if p.exists() {
                    agents.push(self.make_claude_code_agent(Some(p.clone())));
                    break;
                }
            }
        }

        // Codex
        #[cfg(target_os = "windows")]
        {
            let cx = PathBuf::from(&home).join("AppData").join("Roaming").join("npm").join("codex.cmd");
            if cx.exists() {
                agents.push(self.make_codex_agent(Some(cx)));
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            for p in &[
                PathBuf::from(&home).join(".local").join("bin").join("codex"),
                PathBuf::from("/usr/local/bin/codex"),
            ] {
                if p.exists() {
                    agents.push(self.make_codex_agent(Some(p.clone())));
                    break;
                }
            }
        }

        // OpenClaw config
        let oc_path = PathBuf::from(&home).join(".openclaw").join("openclaw.json");
        if oc_path.exists() {
            if let Some(agent) = self.try_read_openclaw(&oc_path) {
                agents.push(agent);
            }
        }

        agents
    }

    fn layer2_which(&self) -> Vec<AgentConfig> {
        let mut agents = Vec::new();
        #[cfg(target_os = "windows")]
        let shells: &[&str] = &["where", "where.exe"];
        #[cfg(not(target_os = "windows"))]
        let shells: &[&str] = &["which"];

        for shell_cmd in shells {
            if let Ok(output) = Command::new(shell_cmd).arg("claude").output() {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path.is_empty() {
                        agents.push(self.make_claude_code_agent(Some(PathBuf::from(path))));
                    }
                }
            }
            if let Ok(output) = Command::new(shell_cmd).arg("codex").output() {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path.is_empty() {
                        agents.push(self.make_codex_agent(Some(PathBuf::from(path))));
                    }
                }
            }
        }

        agents
    }

    fn layer3_shell_probe(&self) -> Vec<AgentConfig> {
        #[cfg(not(target_os = "windows"))]
        {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            let oc = PathBuf::from(&home).join(".openclaw").join("openclaw.json");
            if oc.exists() {
                if let Some(agent) = self.try_read_openclaw(&oc) {
                    return vec![agent];
                }
            }
        }
        Vec::new()
    }

    fn try_read_openclaw(&self, path: &PathBuf) -> Option<AgentConfig> {
        let raw = std::fs::read_to_string(path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&raw).ok()?;

        // Token can be at: gateway.auth.token (newer OpenClaw) or api_key/token root (legacy)
        let api_key = v["gateway"]["auth"]["token"]
            .as_str()
            .or(v["api_key"].as_str())
            .or(v["token"].as_str())
            .map(String::from);

        // Port can be at: gateway.port (newer) or port root (legacy)
        let port = v["gateway"]["port"]
            .as_u64()
            .or(v["port"].as_u64())
            .unwrap_or(18789);

        let models: Vec<String> = v["models"]
            .as_array()
            .map(|a| a.iter().filter_map(|m| m.as_str().map(String::from)).collect())
            .unwrap_or_else(|| vec!["openclaw".into()]);

        Some(AgentConfig {
            id: "openclaw".into(),
            display_name: "OpenClaw".into(),
            agent_type: "openclaw".into(),
            enabled: true,
            config: AgentConnectionConfig {
                base_url: format!("http://127.0.0.1:{port}/v1"),
                api_key,
                models,
                vision_models: vec![],
                reasoning_models: vec![],
                ..Default::default()
            },
            detected: Some(true),
            detected_path: Some(path.to_string_lossy().to_string()),
            added_manually: None,
        })
    }

    fn make_claude_code_agent(&self, path: Option<PathBuf>) -> AgentConfig {
        AgentConfig {
            id: "claude-code".into(),
            display_name: "Claude Code".into(),
            agent_type: "claude_code".into(),
            enabled: false,
            config: AgentConnectionConfig {
                base_url: String::new(),
                api_key: None,
                models: vec!["claude-sonnet-4-6".into()],
                vision_models: vec![],
                reasoning_models: vec![],
                ..Default::default()
            },
            detected: Some(true),
            detected_path: path.map(|p| p.to_string_lossy().to_string()),
            added_manually: None,
        }
    }

    fn make_codex_agent(&self, path: Option<PathBuf>) -> AgentConfig {
        AgentConfig {
            id: "codex".into(),
            display_name: "OpenAI Codex".into(),
            agent_type: "codex".into(),
            enabled: false,
            config: AgentConnectionConfig {
                base_url: String::new(),
                api_key: None,
                models: vec!["gpt-4o".into()],
                vision_models: vec![],
                reasoning_models: vec![],
                ..Default::default()
            },
            detected: Some(true),
            detected_path: path.map(|p| p.to_string_lossy().to_string()),
            added_manually: None,
        }
    }
}
