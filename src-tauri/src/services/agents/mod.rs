pub mod agent_interface;
pub mod hermes_builtin;
pub mod nexus_tools;
pub mod openai_compatible;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tokio::sync::RwLock;

use super::agent_detector::AgentDetector;
use super::agent_store::AgentStore;
use agent_interface::AgentInterface;

/// Deserialized from agents.json — raw config for a single agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: String,
    pub display_name: String,
    pub agent_type: String,
    #[serde(default)]
    pub enabled: bool,
    pub config: AgentConnectionConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_manually: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConnectionConfig {
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub vision_models: Vec<String>,
    #[serde(default)]
    pub reasoning_models: Vec<String>,
}

/// Sanitized agent info for the frontend (no api_key).
#[derive(Debug, Clone, Serialize)]
pub struct AgentInfo {
    pub id: String,
    pub display_name: String,
    pub agent_type: String,
    pub enabled: bool,
    pub models: Vec<String>,
    pub vision_models: Vec<String>,
    pub reasoning_models: Vec<String>,
    pub healthy: bool,
    pub detected: bool,
    pub added_manually: bool,
    pub status: AgentStatus,
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum AgentStatus {
    Running,
    Detected,
    Offline,
}

/// The agent registry holds all agent instances, the store, and the detector.
/// It is the single entry-point for agent lifecycle management.
pub struct AgentRegistry {
    store: AgentStore,
    detector: AgentDetector,
    instances: RwLock<HashMap<String, Box<dyn AgentInterface>>>,
    previous_health: RwLock<HashMap<String, bool>>,
}

impl AgentRegistry {
    pub fn new(hermes_home: &std::path::Path) -> Self {
        Self {
            store: AgentStore::new(hermes_home),
            detector: AgentDetector::new(),
            instances: RwLock::new(HashMap::new()),
            previous_health: RwLock::new(HashMap::new()),
        }
    }

    /// Load persisted agents from agents.json, seed if missing.
    pub fn load_persisted(&self) -> Result<Vec<AgentConfig>, String> {
        self.store.load_or_seed()
    }

    /// Background scan for locally installed agents (Claude Code, Codex, OpenClaw).
    pub async fn background_scan(&self) -> Vec<AgentConfig> {
        self.detector.scan_all().await
    }

    /// Register an agent instance into the runtime registry.
    pub async fn register(&self, id: &str, agent: Box<dyn AgentInterface>) {
        self.instances.write().await.insert(id.to_string(), agent);
    }

    /// Remove an agent instance from the runtime registry.
    pub async fn unregister(&self, id: &str) {
        self.instances.write().await.remove(id);
    }

    /// Get agent info list (sanitized for frontend).
    /// Runs a quick health probe against each enabled agent to verify connectivity.
    pub async fn list(&self) -> Vec<AgentInfo> {
        let configs = self.store.load_or_seed().unwrap_or_default();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap_or_default();

        let mut infos = Vec::new();
        for c in configs {
            let healthy = if c.enabled && !c.config.base_url.is_empty() {
                probe_health(&client, &c.config.base_url, &c.agent_type, c.config.api_key.as_deref()).await
            } else {
                false
            };
            let detected = c.detected.unwrap_or(false);
            let cid = c.id.clone();
            infos.push(AgentInfo {
                healthy,
                id: cid,
                display_name: c.display_name,
                agent_type: c.agent_type,
                enabled: c.enabled,
                models: c.config.models,
                vision_models: c.config.vision_models,
                reasoning_models: c.config.reasoning_models,
                detected,
                added_manually: c.added_manually.unwrap_or(false),
                status: if healthy {
                    AgentStatus::Running
                } else if detected {
                    AgentStatus::Detected
                } else {
                    AgentStatus::Offline
                },
                base_url: c.config.base_url,
            });
        }
        infos
    }

    /// Add a manually configured agent and persist it.
    pub fn add_manual(&self, config: AgentConfig) -> Result<(), String> {
        let mut configs = self.store.load_or_seed()?;
        configs.retain(|c| c.id != config.id);
        configs.push(config);
        self.store.save(&configs)
    }

    /// Update an existing agent's config (base_url, api_key, models).
    pub fn update_config(
        &self,
        id: &str,
        base_url: Option<String>,
        api_key: Option<String>,
        models: Option<Vec<String>>,
        vision_models: Option<Vec<String>>,
        reasoning_models: Option<Vec<String>>,
    ) -> Result<(), String> {
        let mut configs = self.store.load_or_seed()?;
        let c = configs
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| format!("Agent not found: {id}"))?;
        if let Some(url) = base_url {
            c.config.base_url = url;
        }
        if let Some(key) = api_key {
            c.config.api_key = if key.is_empty() { None } else { Some(key) };
        }
        if let Some(m) = models {
            c.config.models = m;
        }
        if let Some(v) = vision_models {
            c.config.vision_models = v;
        }
        if let Some(r) = reasoning_models {
            c.config.reasoning_models = r;
        }
        self.store.save(&configs)
    }

    /// Remove an agent by id.
    pub fn remove(&self, id: &str) -> Result<(), String> {
        let mut configs = self.store.load_or_seed()?;
        configs.retain(|c| c.id != id);
        self.store.save(&configs)
    }

    /// Enable or disable an agent.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        let mut configs = self.store.load_or_seed()?;
        if let Some(c) = configs.iter_mut().find(|c| c.id == id) {
            c.enabled = enabled;
        }
        self.store.save(&configs)
    }

    /// Set the default agent.
    pub fn set_default(&self, id: &str) -> Result<(), String> {
        let agents = self.store.load_or_seed()?;
        let root = serde_json::json!({
            "agents": &agents,
            "default_agent_id": id
        });
        let json = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
        let tmp = self.store.path().with_extension("json.tmp");
        std::fs::write(&tmp, &json).map_err(|e| format!("写入 agents.json 临时文件失败: {e}"))?;
        std::fs::rename(&tmp, self.store.path()).map_err(|e| format!("替换 agents.json 失败: {e}"))
    }

    /// Get the default agent id.
    pub fn default_agent_id(&self) -> Option<String> {
        self.store.default_agent_id()
    }

    /// Get the raw store reference.
    pub fn store(&self) -> &AgentStore {
        &self.store
    }

    /// Periodic health tick — re-probes all agents and returns true if any status changed.
    /// Used by the background health poller to emit `agents:updated` only on actual changes.
    pub async fn tick_health(&self) -> bool {
        let agents = self.list().await;
        let mut prev = self.previous_health.write().await;
        let mut changed = false;

        let current_ids: HashSet<String> = agents.iter().map(|a| a.id.clone()).collect();
        for a in &agents {
            let old = prev.get(&a.id).copied().unwrap_or(!a.healthy);
            if old != a.healthy {
                log::info!(
                    "[health_poller] {} healthy: {} -> {}",
                    a.id, old, a.healthy
                );
                changed = true;
            }
            prev.insert(a.id.clone(), a.healthy);
        }
        prev.retain(|id, _| current_ids.contains(id));
        changed
    }

    /// Merge detected agents into persisted config.
    pub fn merge_detected(&self, detected: &[AgentConfig]) -> Result<(), String> {
        self.store.merge_detected(detected)
    }
}

/// Quick HTTP probe to check if an agent is reachable.
/// For hermes_builtin: strips /v1 suffix and checks /health.
/// For openclaw: POSTs to /v1/chat/completions to verify the HTTP API is enabled.
/// For openai_compatible: checks /models with optional API key.
async fn probe_health(
    client: &reqwest::Client,
    base_url: &str,
    agent_type: &str,
    api_key: Option<&str>,
) -> bool {
    let (method_is_post, health_url) = match agent_type {
        "hermes_builtin" => {
            let stripped = base_url.strip_suffix("/v1").unwrap_or(base_url);
            (false, format!("{stripped}/health"))
        }
        "openclaw" => {
            // OpenClaw: chat completions HTTP API is disabled by default.
            // Probe the actual chat endpoint (POST) so that a 404 means "not usable".
            (true, format!("{base_url}/chat/completions"))
        }
        _ => (false, format!("{base_url}/models")),
    };

    let result = if method_is_post {
        // Minimal POST — if endpoint is disabled, OpenClaw returns 404.
        let body = serde_json::json!({"model": "__health_probe__", "messages": []});
        let mut req = client.post(&health_url).json(&body);
        if let Some(key) = api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        req.send().await
    } else {
        let mut req = client.get(&health_url);
        if let Some(key) = api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        req.send().await
    };

    match result {
        Ok(resp) => {
            let status = resp.status();
            // openclaw returns 404 if the HTTP API endpoint is disabled.
            // 401 means auth is missing or wrong — not healthy either.
            if agent_type == "openclaw" {
                let ok = status.as_u16() != 404 && status.as_u16() != 401;
                if ok {
                    log::info!("[probe_health] {base_url} chat endpoint reachable (HTTP {status})");
                } else if status.as_u16() == 401 {
                    log::warn!("[probe_health] {base_url} chat endpoint 401 — API key missing or invalid");
                } else {
                    log::warn!("[probe_health] {base_url} chat endpoint 404 — HTTP API likely disabled");
                }
                ok
            } else {
                let ok = status.is_success();
                if ok {
                    log::info!("[probe_health] {base_url} OK ({health_url})");
                } else {
                    log::warn!("[probe_health] {base_url} returned {status} ({health_url})");
                }
                ok
            }
        }
        Err(e) => {
            log::warn!("[probe_health] {base_url} unreachable: {e}");
            false
        }
    }
}
