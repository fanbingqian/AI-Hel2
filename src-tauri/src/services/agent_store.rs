use std::fs;
use std::path::{Path, PathBuf};

use super::agents::AgentConfig;

pub struct AgentStore {
    path: PathBuf,
    default_agent_id: Option<String>,
}

impl AgentStore {
    pub fn new(hermes_home: &Path) -> Self {
        Self { path: hermes_home.join("agents.json"), default_agent_id: None }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Load agents.json. If it doesn't exist or is corrupted, seed with a Hermes builtin record.
    pub fn load_or_seed(&self) -> Result<Vec<AgentConfig>, String> {
        if self.path.exists() {
            let raw = match fs::read_to_string(&self.path) {
                Ok(r) => r,
                Err(e) => {
                    log::error!("Failed to read agents.json: {e}");
                    return self.seed();
                }
            };
            let root: serde_json::Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => {
                    // Corrupted JSON — back up and re-seed
                    log::warn!("agents.json parse failed ({}): re-seeding from defaults", e);
                    let backup = self.path.with_extension("json.bak");
                    let _ = fs::remove_file(&backup);
                    let _ = fs::rename(&self.path, &backup);
                    return self.seed();
                }
            };
            let agents: Vec<AgentConfig> = match serde_json::from_value(root["agents"].clone()) {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("agents.json deserialize failed ({}): re-seeding", e);
                    return self.seed();
                }
            };
            if agents.is_empty() {
                return self.seed();
            }
            Ok(agents)
        } else {
            self.seed()
        }
    }

    fn seed(&self) -> Result<Vec<AgentConfig>, String> {
        let seed = AgentConfig {
            id: "hermes-builtin".into(),
            display_name: "Hermes Agent (内置)".into(),
            agent_type: "hermes_builtin".into(),
            enabled: true,
            config: super::agents::AgentConnectionConfig {
                base_url: "http://127.0.0.1:18642/v1".into(),
                api_key: None,
                models: vec!["claude-sonnet-4-6".into(), "deepseek-v4-pro".into()],
                vision_models: vec![],
                reasoning_models: vec![],
                ..Default::default()
            },
            detected: None,
            detected_path: None,
            added_manually: None,
        };
        let agents = vec![seed];
        self.save(&agents)?;
        Ok(agents)
    }

    /// Persist the full agent list to agents.json (atomic write via temp file).
    pub fn save(&self, agents: &[AgentConfig]) -> Result<(), String> {
        let root = serde_json::json!({
            "agents": agents,
            "default_agent_id": self.default_agent_id.as_deref().unwrap_or("hermes-builtin")
        });
        let json = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, &json).map_err(|e| format!("写入 agents.json 临时文件失败: {e}"))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("替换 agents.json 失败: {e}"))
    }

    /// Set the default agent id.
    pub fn set_default(&mut self, id: &str) -> Result<(), String> {
        self.default_agent_id = Some(id.to_string());
        let agents = self.load_or_seed()?;
        self.save(&agents)
    }

    /// Get the default agent id.
    pub fn default_agent_id(&self) -> Option<String> {
        self.default_agent_id.clone()
    }

    /// Merge detected agents: new ones are appended, existing ones updated.
    pub fn merge_detected(&self, detected: &[AgentConfig]) -> Result<(), String> {
        let mut configs = self.load_or_seed()?;
        for d in detected {
            if let Some(existing) = configs.iter_mut().find(|c| c.id == d.id) {
                existing.detected = Some(true);
                existing.detected_path = d.detected_path.clone();
                // Restore base_url and api_key from detection (may have been corrupted)
                if !d.config.base_url.is_empty() {
                    existing.config.base_url = d.config.base_url.clone();
                }
                if d.config.api_key.is_some() {
                    existing.config.api_key = d.config.api_key.clone();
                }
                if !d.config.models.is_empty() {
                    existing.config.models = d.config.models.clone();
                }
                if !d.config.vision_models.is_empty() {
                    existing.config.vision_models = d.config.vision_models.clone();
                }
                if !d.config.reasoning_models.is_empty() {
                    existing.config.reasoning_models = d.config.reasoning_models.clone();
                }
            } else {
                configs.push(d.clone());
            }
        }
        self.save(&configs)
    }
}
