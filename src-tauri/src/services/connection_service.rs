use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum ConnectionMode {
    Local,
    Remote,
    Ssh,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionConfig {
    pub mode: ConnectionMode,
    pub agent_url: String,
    pub api_key: Option<String>,
    pub remote_host: Option<String>,
    pub remote_port: Option<u16>,
    pub ssh_user: Option<String>,
    pub ssh_key_path: Option<String>,
    pub ssh_passphrase: Option<String>,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            mode: ConnectionMode::Local,
            agent_url: "http://127.0.0.1:18642".into(),
            api_key: None,
            remote_host: None,
            remote_port: None,
            ssh_user: None,
            ssh_key_path: None,
            ssh_passphrase: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthStatus {
    pub healthy: bool,
    pub version: Option<String>,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

pub struct ConnectionService {
    pub config: ConnectionConfig,
    client: reqwest::Client,
}

impl ConnectionService {
    pub fn new() -> Self {
        Self {
            config: ConnectionConfig::default(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn new_with_url(url: &str) -> Self {
        let mut config = ConnectionConfig::default();
        config.agent_url = url.to_string();
        Self {
            config,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn agent_url(&self) -> &str {
        &self.config.agent_url
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub fn set_config(&mut self, config: ConnectionConfig) {
        self.config = config;
    }

    pub async fn check_health(&self) -> HealthStatus {
        let start = std::time::Instant::now();
        let url = format!("{}/health", self.config.agent_url);

        match self.client.get(&url).send().await {
            Ok(resp) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                if resp.status().is_success() {
                    let version = resp
                        .headers()
                        .get("x-hermes-version")
                        .and_then(|v| v.to_str().ok())
                        .map(String::from);

                    HealthStatus {
                        healthy: true,
                        version,
                        latency_ms: Some(latency_ms),
                        error: None,
                    }
                } else {
                    HealthStatus {
                        healthy: false,
                        version: None,
                        latency_ms: Some(latency_ms),
                        error: Some(format!("HTTP {}", resp.status())),
                    }
                }
            }
            Err(e) => HealthStatus {
                healthy: false,
                version: None,
                latency_ms: None,
                error: Some(e.to_string()),
            },
        }
    }

    /// Test remote connection — verifies a URL is reachable
    pub async fn test_remote_connection(&self, url: &str) -> Result<HealthStatus, String> {
        let start = std::time::Instant::now();
        let health_url = format!("{url}/health");

        self.client
            .get(&health_url)
            .send()
            .await
            .map(|resp| {
                let latency_ms = start.elapsed().as_millis() as u64;
                let version = resp
                    .headers()
                    .get("x-hermes-version")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);
                HealthStatus {
                    healthy: resp.status().is_success(),
                    version,
                    latency_ms: Some(latency_ms),
                    error: if resp.status().is_success() {
                        None
                    } else {
                        Some(format!("HTTP {}", resp.status()))
                    },
                }
            })
            .map_err(|e| format!("远程连接失败: {e}"))
    }

    /// Stub: SSH tunnel setup.
    /// In Phase 2 this will use the `ssh2` crate for real SSH tunnelling.
    /// For now it validates config and reports readiness.
    pub fn validate_ssh_config(&self) -> Result<(), String> {
        let host = self
            .config
            .remote_host
            .as_ref()
            .ok_or("SSH 主机未配置")?;
        let port = self.config.remote_port.unwrap_or(22);
        let user = self.config.ssh_user.as_deref().ok_or("SSH 用户未配置")?;
        let _key_path = self
            .config
            .ssh_key_path
            .as_deref()
            .ok_or("SSH 密钥路径未配置")?;

        log::info!(
            "SSH config validated: {user}@{host}:{port} (Phase 2 will establish tunnel)"
        );
        Ok(())
    }
}
