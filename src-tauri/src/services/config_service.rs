use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HermesConfig {
    pub model: ModelConfig,
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub active_profile: Option<String>,
    #[serde(default)]
    pub appearance: Option<AppearanceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceConfig {
    #[serde(default = "default_theme")]
    pub theme: String,
}

fn default_theme() -> String { "system".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_model")]
    pub name: String,
    /// Gateway model name — used by the Hermes gateway's _resolve_gateway_model()
    /// which reads model.default (not model.name) from config.yaml.
    #[serde(default = "default_gateway_model")]
    pub default: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_provider() -> String { "anthropic".into() }
fn default_model() -> String { "claude-sonnet-4-6".into() }
fn default_gateway_model() -> String { "deepseek-v4-flash".into() }
fn default_port() -> u16 { 18642 }

pub struct ConfigService {
    hermes_home: PathBuf,
}

impl ConfigService {
    pub fn new() -> Self {
        let home = dirs_ai_hel2_home();
        Self { hermes_home: home }
    }

    pub fn hermes_home(&self) -> &Path {
        &self.hermes_home
    }

    pub fn read_config(&self) -> Result<HermesConfig, String> {
        let config_path = self.hermes_home.join("config.yaml");
        if !config_path.exists() {
            return Ok(HermesConfig::default());
        }

        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("读取 config.yaml 失败: {e}"))?;
        let content = strip_bom(&content);

        serde_yaml::from_str(&content)
            .map_err(|e| format!("解析 config.yaml 失败: {e}"))
    }

    pub fn write_config(&self, updates: &serde_json::Value) -> Result<(), String> {
        let config = self.read_config()?;
        let config_value = serde_json::to_value(&config)
            .map_err(|e| format!("序列化配置失败: {e}"))?;

        let merged = merge_json_values(&config_value, updates);

        let yaml = serde_yaml::to_string(&merged)
            .map_err(|e| format!("序列化 YAML 失败: {e}"))?;

        let config_path = self.hermes_home.join("config.yaml");
        self.atomic_write(&config_path, &yaml)
    }

    pub fn read_env(&self) -> HashMap<String, String> {
        let env_path = self.hermes_home.join(".env");
        if !env_path.exists() {
            return HashMap::new();
        }

        dotenvy::from_filename_iter(env_path)
            .map(|iter| iter.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    }

    /// Read the nexus section from config.yaml.
    pub fn read_nexus_config(&self) -> Result<serde_json::Value, String> {
        let config_path = self.hermes_home.join("config.yaml");
        if !config_path.exists() {
            return Ok(serde_json::json!({"llm_mode": "follow_agent"}));
        }
        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("读取 config.yaml 失败: {e}"))?;
        let content = strip_bom(&content);
        let value: serde_yaml::Value = serde_yaml::from_str(&content)
            .map_err(|e| format!("解析 config.yaml 失败: {e}"))?;
        let nexus = value.get("nexus")
            .map(|n| serde_json::to_value(n).unwrap_or_default())
            .unwrap_or_else(|| serde_json::json!({"llm_mode": "follow_agent"}));
        Ok(nexus)
    }

    /// Write the nexus section to config.yaml, preserving other sections.
    pub fn write_nexus_config(&self, nexus: &serde_json::Value) -> Result<(), String> {
        let config_path = self.hermes_home.join("config.yaml");
        let mut root: serde_yaml::Value = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .map_err(|e| format!("读取 config.yaml 失败: {e}"))?;
            let content = strip_bom(&content);
            serde_yaml::from_str(&content).unwrap_or(serde_yaml::Value::Mapping(Default::default()))
        } else {
            serde_yaml::Value::Mapping(Default::default())
        };

        let nexus_yaml: serde_yaml::Value = serde_json::from_value(nexus.clone())
            .map_err(|e| format!("转换 nexus 配置失败: {e}"))?;

        if let Some(mapping) = root.as_mapping_mut() {
            mapping.insert(
                serde_yaml::Value::String("nexus".into()),
                nexus_yaml,
            );
        }

        let yaml = serde_yaml::to_string(&root)
            .map_err(|e| format!("序列化 YAML 失败: {e}"))?;
        self.atomic_write(&config_path, &yaml)
    }

    /// Collect LLM env vars to pass to extract_service.py subprocess.
    pub fn nexus_env_vars(&self) -> Vec<(String, String)> {
        let nexus = self.read_nexus_config().unwrap_or_default();
        let llm_mode = nexus.get("llm_mode").and_then(|v| v.as_str()).unwrap_or("follow_agent");

        if llm_mode == "custom" {
            let provider = nexus.get("llm_provider").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("anthropic");
            let model = nexus.get("llm_model").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("claude-sonnet-4-6");
            let api_key = nexus.get("llm_api_key").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("");
            let base_url = nexus.get("llm_base_url").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("");

            vec![
                ("NEXUS_LLM_MODE".into(), "custom".into()),
                ("NEXUS_LLM_PROVIDER".into(), provider.into()),
                ("NEXUS_LLM_MODEL".into(), model.into()),
                ("NEXUS_LLM_API_KEY".into(), api_key.into()),
                ("NEXUS_LLM_BASE_URL".into(), base_url.into()),
            ]
        } else {
            // follow_agent: inherit LLM config from the main agent
            let model_config = self.read_config().unwrap_or_default();
            let mut vars = vec![
                ("NEXUS_LLM_MODE".into(), "follow_agent".into()),
            ];

            // When using Hermes builtin agent, route Nexus LLM calls through Hermes
            if model_config.model.provider == "hermes-builtin" {
                vars.push(("NEXUS_LLM_PROVIDER".into(), "hermes_builtin".into()));
                vars.push(("NEXUS_LLM_MODEL".into(), model_config.model.name.clone()));
                vars.push(("NEXUS_LLM_BASE_URL".into(), "http://127.0.0.1:18642/v1".into()));
                return vars;
            }

            // Pass provider-specific keys from .env
            let env = self.read_env();
            let mut provider = String::new();
            let mut api_key_val = String::new();
            for (key, val) in &env {
                if key.ends_with("_API_KEY") && !val.is_empty() {
                    provider = key.trim_end_matches("_API_KEY").to_lowercase();
                    api_key_val = val.clone();
                    vars.push(("NEXUS_LLM_PROVIDER".into(), provider.clone()));
                    vars.push(("NEXUS_LLM_API_KEY".into(), api_key_val));
                    break;
                }
            }
            let model_name = if model_config.model.name.is_empty() {
                match provider.as_str() {
                    "deepseek" => "deepseek-v4-pro".to_string(),
                    "openai" => "gpt-4o".to_string(),
                    _ => "claude-sonnet-4-6".to_string(),
                }
            } else {
                model_config.model.name
            };
            vars.push(("NEXUS_LLM_MODEL".into(), model_name));
            if provider.is_empty() {
                vars.push(("NEXUS_LLM_PROVIDER".into(), model_config.model.provider));
                if let Some(key) = model_config.model.api_key {
                    vars.push(("NEXUS_LLM_API_KEY".into(), key));
                }
            }
            vars
        }
    }

    pub fn set_env(&self, key: &str, value: &str) -> Result<(), String> {
        let mut env = self.read_env();
        env.insert(key.to_string(), value.to_string());

        let content: String = env
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");

        let env_path = self.hermes_home.join(".env");
        self.atomic_write(&env_path, &content)
    }

    pub fn atomic_write(&self, path: &Path, content: &str) -> Result<(), String> {
        let tmp_path = path.with_extension("tmp");
        std::fs::write(&tmp_path, content)
            .map_err(|e| format!("写入临时文件失败: {e}"))?;
        std::fs::rename(&tmp_path, path)
            .map_err(|e| format!("替换文件失败: {e}"))
    }
}

impl Default for HermesConfig {
    fn default() -> Self {
        Self {
            model: ModelConfig {
                provider: default_provider(),
                name: default_model(),
                default: default_gateway_model(),
                api_key: None,
            },
            gateway: GatewayConfig { port: default_port() },
            active_profile: None,
            appearance: Some(AppearanceConfig { theme: default_theme() }),
        }
    }
}

/// Resolve the AI-Hel2 data directory.
///
/// Priority:
/// 1. `AI_HEL2_HOME` env var — explicit user override
/// 2. Portable mode — if `{exe_dir}/data` exists, use it (installed version)
/// 3. Default — `{USERPROFILE}/.ai-hel2` (development / legacy)
pub fn dirs_ai_hel2_home() -> PathBuf {
    // 1. Explicit env var override
    if let Ok(home) = std::env::var("AI_HEL2_HOME") {
        let p = PathBuf::from(&home);
        if p.is_absolute() {
            return p;
        }
    }
    // 2. Portable mode: NSIS generates uninstall.exe next to the binary — this
    //    is the most reliable signal that we are in an installed (not dev) copy.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            if exe_dir.join("uninstall.exe").exists() {
                let portable_data = exe_dir.join("data");
                let _ = std::fs::create_dir_all(&portable_data);
                log::info!("Portable mode: data dir at {}", portable_data.display());
                return portable_data;
            }
        }
    }
    // 3. Default: ~/.ai-hel2
    let home = dirs_home();
    home.join(".ai-hel2")
}

pub fn dirs_home() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    }
}

fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{FEFF}').unwrap_or(s)
}

fn merge_json_values(base: &serde_json::Value, updates: &serde_json::Value) -> serde_json::Value {
    match (base, updates) {
        (serde_json::Value::Object(b), serde_json::Value::Object(u)) => {
            let mut merged = b.clone();
            for (k, v) in u {
                match (b.get(k), v) {
                    // Deep-merge nested objects so partial updates
                    // don't strip sibling keys (e.g. model.default).
                    (Some(serde_json::Value::Object(_)), serde_json::Value::Object(_)) => {
                        merged.insert(k.clone(), merge_json_values(&b[k], v));
                    }
                    _ => {
                        merged.insert(k.clone(), v.clone());
                    }
                }
            }
            serde_json::Value::Object(merged)
        }
        (_, u) => u.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_default_config() {
        let config = HermesConfig::default();
        assert_eq!(config.model.provider, "anthropic");
        assert_eq!(config.gateway.port, 18642);
    }
}
