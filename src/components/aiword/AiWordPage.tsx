import { useState, useEffect, useCallback } from "react";
import styles from "./AiWordPage.module.css";

const STORAGE_KEY = "aiword_connection";

interface ConnectionConfig {
  url: string;
  token: string;
}

function loadConfig(): ConnectionConfig | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw);
  } catch {}
  return null;
}

function saveConfig(cfg: ConnectionConfig) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(cfg));
}

function clearConfig() {
  localStorage.removeItem(STORAGE_KEY);
}

export default function AiWordPage() {
  const [connected, setConnected] = useState(false);
  const [url, setUrl] = useState("http://127.0.0.1:3000");
  const [token, setToken] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [iframeKey, setIframeKey] = useState(0);

  useEffect(() => {
    const cfg = loadConfig();
    if (cfg) {
      setUrl(cfg.url);
      setToken(cfg.token);
      setConnected(true);
    }
  }, []);

  const handleConnect = useCallback(() => {
    const trimmed = url.trim();
    if (!trimmed) {
      setError("请输入 Claw3D 服务地址");
      return;
    }
    try {
      new URL(trimmed);
    } catch {
      setError("无效的服务地址");
      return;
    }
    setError(null);
    saveConfig({ url: trimmed, token: token.trim() });
    setConnected(true);
    setIframeKey((k) => k + 1);
  }, [url, token]);

  const handleDisconnect = useCallback(() => {
    clearConfig();
    setConnected(false);
    setError(null);
  }, []);

  if (!connected) {
    return (
      <div className={styles.container}>
        <div className={styles.connectPanel}>
          <h2 className={styles.title}>AI Word</h2>
          <p className={styles.desc}>连接到 Claw3D 服务，进入 AI Agent 3D 虚拟空间</p>
          <div className={styles.field}>
            <label className={styles.label}>Claw3D 服务地址</label>
            <input
              className={styles.input}
              type="text"
              value={url}
              onChange={(e) => { setUrl(e.target.value); setError(null); }}
              placeholder="http://127.0.0.1:3000"
              onKeyDown={(e) => e.key === "Enter" && handleConnect()}
            />
          </div>
          <div className={styles.field}>
            <label className={styles.label}>访问 Token（选填）</label>
            <input
              className={styles.input}
              type="password"
              value={token}
              onChange={(e) => { setToken(e.target.value); setError(null); }}
              placeholder="STUDIO_ACCESS_TOKEN"
              onKeyDown={(e) => e.key === "Enter" && handleConnect()}
            />
          </div>
          {error && <div className={styles.error}>{error}</div>}
          <button type="button" className={styles.connectBtn} onClick={handleConnect}>连接</button>
        </div>
      </div>
    );
  }

  return (
    <div className={styles.fullContainer}>
      <iframe
        key={iframeKey}
        className={styles.iframe}
        src={`${url}?token=${encodeURIComponent(token)}`}
        allow="clipboard-read; clipboard-write; autoplay"
        title="AI Word"
      />
      <button type="button" className={styles.floatDisconnect} onClick={handleDisconnect} title="断开连接">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <path d="M18.36 6.64a9 9 0 1 1-12.73 0" />
          <line x1="12" y1="2" x2="12" y2="12" />
        </svg>
      </button>
    </div>
  );
}
