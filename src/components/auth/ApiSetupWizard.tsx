import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAuthStore } from "../../stores/authStore";
import { updateApiKey, saveConfig } from "../../services/api";
import styles from "./AuthForms.module.css";

interface ProviderDef {
  id: string;
  name: string;
  defaultModel: string;
  color: string;
  envKey: string;
}

const PROVIDERS: ProviderDef[] = [
  { id: "anthropic", name: "Anthropic", defaultModel: "claude-sonnet-4-6", color: "#d47642", envKey: "ANTHROPIC_API_KEY" },
  { id: "openai", name: "OpenAI", defaultModel: "gpt-4o", color: "#74a69d", envKey: "OPENAI_API_KEY" },
  { id: "deepseek", name: "DeepSeek", defaultModel: "deepseek-v4-pro", color: "#537de9", envKey: "DEEPSEEK_API_KEY" },
  { id: "google", name: "Google AI", defaultModel: "gemini-2.5-pro", color: "#4285f4", envKey: "GOOGLE_API_KEY" },
  { id: "xai", name: "xAI", defaultModel: "grok-3", color: "#e5e7eb", envKey: "XAI_API_KEY" },
  { id: "openrouter", name: "OpenRouter", defaultModel: "openai/gpt-4o", color: "#a78bfa", envKey: "OPENROUTER_API_KEY" },
  { id: "groq", name: "Groq", defaultModel: "llama-4-maverick", color: "#f97316", envKey: "GROQ_API_KEY" },
  { id: "aigocode", name: "AIGoCode", defaultModel: "gpt-5.4", color: "#07c160", envKey: "AIGOCODE_API_KEY" },
  { id: "custom", name: "自定义", defaultModel: "gpt-4o", color: "#9ca3af", envKey: "CUSTOM_API_KEY" },
];

const AGENT_BASE_URLS: Record<string, string> = {
  aigocode: "https://api.aigocode.com/v1",
  openai: "https://api.openai.com/v1",
  deepseek: "https://api.deepseek.com/v1",
  xai: "https://api.x.ai/v1",
  openrouter: "https://openrouter.ai/api/v1",
  groq: "https://api.groq.com/openai/v1",
};

interface BackendAgent {
  id: string;
  display_name: string;
  agent_type: string;
  enabled: boolean;
  models: string[];
  healthy: boolean;
  detected: boolean;
  added_manually: boolean;
  status: string;
  base_url: string;
}

interface Step2Agent {
  id: string;
  name: string;
  agentType: string;
  baseUrl: string;
  models: string[];
  status: "ready" | "detected" | "offline" | "pending";
  color: string;
}

export function ApiSetupWizard() {
  const [keys, setKeys] = useState<Record<string, string>>({});
  const [activeProvider, setActiveProvider] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [wizardStep, setWizardStep] = useState(1);
  const [detecting, setDetecting] = useState(false);
  const [step2Agents, setStep2Agents] = useState<Step2Agent[]>([]);
  const setStage = useAuthStore((s) => s.setStage);

  const filledCount = Object.values(keys).filter((v) => v.trim().length > 0).length;

  const handleKeyChange = (id: string, value: string) => {
    setKeys((prev) => ({ ...prev, [id]: value }));
    setError(null);
  };

  const buildStep2Agents = useCallback((backendAgents: BackendAgent[]) => {
    const result: Step2Agent[] = [];
    const seen = new Set<string>();

    // 1. Backend agents (Hermes builtin + detected)
    for (const ba of backendAgents) {
      seen.add(ba.id);
      result.push({
        id: ba.id,
        name: ba.display_name,
        agentType: ba.agent_type,
        baseUrl: ba.base_url || "",
        models: [...ba.models],
        status: ba.status === "Running" ? "ready" : ba.status === "Detected" ? "detected" : "offline",
        color: ba.agent_type === "hermes_builtin" ? "#07c160" : "#a78bfa",
      });
    }

    // 2. Pending agents from Step 1 keys
    for (const provider of PROVIDERS) {
      const key = keys[provider.id]?.trim();
      if (!key) continue;
      const agentId = provider.id;
      if (seen.has(agentId)) continue;
      seen.add(agentId);

      const baseUrl = AGENT_BASE_URLS[provider.id] || "";
      result.push({
        id: agentId,
        name: provider.name,
        agentType: "openai_compatible",
        baseUrl,
        models: [provider.defaultModel],
        status: "pending",
        color: provider.color,
      });
    }

    return result;
  }, [keys]);

  const loadStep2 = useCallback(async () => {
    setDetecting(true);
    try {
      // Trigger detection then list all agents
      await invoke("re_detect_agents");
      const backendAgents = await invoke<BackendAgent[]>("list_agents");
      setStep2Agents(buildStep2Agents(backendAgents));
    } catch {
      // If detection fails, still show pending agents from keys
      setStep2Agents(buildStep2Agents([]));
    }
    setDetecting(false);
  }, [buildStep2Agents]);

  const handleNext = () => {
    setWizardStep(2);
  };

  const handleBack = () => {
    setWizardStep(1);
  };

  const handleAddModel = (agentId: string, model: string) => {
    if (!model.trim()) return;
    setStep2Agents((prev) =>
      prev.map((a) =>
        a.id === agentId && !a.models.includes(model.trim())
          ? { ...a, models: [...a.models, model.trim()] }
          : a
      )
    );
  };

  const handleRemoveModel = (agentId: string, model: string) => {
    setStep2Agents((prev) =>
      prev.map((a) =>
        a.id === agentId ? { ...a, models: a.models.filter((m) => m !== model) } : a
      )
    );
  };

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      // 1. Save API keys
      for (const provider of PROVIDERS) {
        const key = keys[provider.id]?.trim();
        if (key) {
          await updateApiKey(provider.id, key);
        }
      }

      // 2. Create agents from Step 2 config
      for (const agent of step2Agents) {
        if (agent.status === "pending") {
          // New agent from Step 1 keys
          const baseUrl = agent.baseUrl || AGENT_BASE_URLS[agent.id] || "";
          const key = keys[agent.id]?.trim() || "";
          try {
            await invoke("add_agent", {
              id: agent.id,
              displayName: agent.name,
              agentType: agent.agentType,
              baseUrl,
              apiKey: key,
              models: agent.models,
            });
          } catch { /* agent may already exist */ }
        } else if (agent.status === "ready" || agent.status === "detected" || agent.status === "offline") {
          // Existing agent — update models if changed (remove + re-add)
          try {
            await invoke("remove_agent", { id: agent.id });
          } catch { /* may not exist */ }
          try {
            const baseUrl = agent.baseUrl || AGENT_BASE_URLS[agent.id] || "";
            await invoke("add_agent", {
              id: agent.id,
              displayName: agent.name,
              agentType: agent.agentType,
              baseUrl,
              apiKey: keys[agent.id]?.trim() || null,
              models: agent.models,
            });
          } catch { /* ignore */ }
        }
      }

      // 3. Agent registered — model config is managed by Hermes
      // (config.yaml / hermes CLI), not by AI-Hel2.

      setStage("done");
    } catch (err: any) {
      const msg = typeof err === "string" ? err : err?.message || JSON.stringify(err);
      setError(`保存失败: ${msg}`);
    } finally {
      setSaving(false);
    }
  };

  const handleSkip = () => {
    setStage("done");
  };

  // Load Step 2 data when entering Step 2
  useEffect(() => {
    if (wizardStep === 2) {
      loadStep2();
    }
  }, [wizardStep, loadStep2]);

  return (
    <div className={styles.overlay}>
      <div className={`${styles.card} ${styles.wideCard}`}>
        {wizardStep === 1 ? (
          <>
            <div className={styles.stepTitle}>配置大模型 API</div>
            <div className={styles.stepDesc}>
              粘贴 API Key 即可开箱使用，与 Hermes Agent 直接对接。配置后同步到<b>系统设置 → 大模型配置</b>，可随时增删。
            </div>

            <div className={styles.provList}>
              {PROVIDERS.map((prov) => {
                const hasKey = keys[prov.id]?.trim().length > 0;
                const isActive = activeProvider === prov.id;
                return (
                  <div
                    key={prov.id}
                    className={`${styles.provRow} ${isActive ? styles.active : ""}`}
                    onClick={() => {
                      setActiveProvider(prov.id);
                      const input = document.getElementById(`key-input-${prov.id}`) as HTMLInputElement;
                      input?.focus();
                    }}
                  >
                    <span className={styles.provDot} style={{ background: prov.color }} />
                    <span className={styles.provName}>{prov.name}</span>
                    <span className={styles.provModel}>{prov.defaultModel}</span>
                    <input
                      id={`key-input-${prov.id}`}
                      name={`api-key-${prov.id}`}
                      className={styles.keyInput}
                      type="password"
                      placeholder="粘贴 API Key..."
                      value={keys[prov.id] || ""}
                      onChange={(e) => handleKeyChange(prov.id, e.target.value)}
                      onFocus={() => setActiveProvider(prov.id)}
                      aria-label={`${prov.name} API Key`}
                    />
                    <span className={`${styles.provStatus} ${hasKey ? styles.done : ""}`}>
                      {hasKey ? "已配置" : "未配置"}
                    </span>
                  </div>
                );
              })}
            </div>

            <div className={styles.readyHint}>
              已配置 <span>{filledCount}</span> 个模型，Hermes Agent 开箱即用
            </div>

            {error && <div className={styles.error}>{error}</div>}

            <div className={styles.btnRow}>
              <button type="button" className={styles.skipLink} onClick={handleSkip}>
                跳过，稍后配置
              </button>
              <button type="button" className={styles.startBtn} onClick={handleNext}>
                下一步
              </button>
            </div>
          </>
        ) : (
          <>
            <div className={styles.stepTitle}>检测本地 Agent</div>
            <div className={styles.stepDesc}>
              以下是在你电脑上发现的 AI Agent，可直接切换使用。展开可配置每个 Agent 的模型列表。
            </div>

            {detecting ? (
              <div className={styles.loadingHint}>正在检测本地 Agent...</div>
            ) : (
              <div className={styles.agentList}>
                {step2Agents.length === 0 && (
                  <div className={styles.emptyHint}>未检测到任何 Agent，请返回上一步配置 API Key</div>
                )}
                {step2Agents.map((agent) => (
                  <AgentCard
                    key={agent.id}
                    agent={agent}
                    onAddModel={(model) => handleAddModel(agent.id, model)}
                    onRemoveModel={(model) => handleRemoveModel(agent.id, model)}
                  />
                ))}
              </div>
            )}

            {error && <div className={styles.error}>{error}</div>}

            <div className={styles.btnRow}>
              <button type="button" className={styles.skipLink} onClick={handleBack}>
                ← 上一步
              </button>
              <button type="button" className={styles.startBtn} onClick={handleSave} disabled={saving || detecting}>
                {saving ? "保存中..." : "开始使用"}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function AgentCard({
  agent,
  onAddModel,
  onRemoveModel,
}: {
  agent: Step2Agent;
  onAddModel: (model: string) => void;
  onRemoveModel: (model: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [newModel, setNewModel] = useState("");

  const statusIcon =
    agent.status === "ready" ? "●" : agent.status === "detected" ? "●" : agent.status === "pending" ? "○" : "○";
  const statusClass =
    agent.status === "ready" ? styles.statusReady : agent.status === "detected" ? styles.statusDetected : agent.status === "pending" ? styles.statusPending : styles.statusOffline;
  const statusLabel =
    agent.status === "ready" ? "已就绪" : agent.status === "detected" ? "已检测" : agent.status === "pending" ? "待创建" : "离线";

  const typeLabel =
    agent.agentType === "hermes_builtin" ? "内置服务 :18642" : agent.baseUrl ? agent.baseUrl.replace(/^https?:\/\//, "") : agent.agentType;

  const handleAdd = () => {
    if (newModel.trim()) {
      onAddModel(newModel.trim());
      setNewModel("");
    }
  };

  return (
    <div className={`${styles.agentCard} ${expanded ? styles.agentCardExpanded : ""}`}>
      <div className={styles.agentCardHeader} onClick={() => setExpanded(!expanded)}>
        <div className={styles.agentCardLeft}>
          <span className={styles.agentDot} style={{ background: agent.color }} />
          <div>
            <div className={styles.agentName}>{agent.name}</div>
            <div className={styles.agentMeta}>{typeLabel}</div>
          </div>
        </div>
        <div className={styles.agentCardRight}>
          <span className={`${styles.agentStatus} ${statusClass}`}>
            {statusIcon} {statusLabel}
          </span>
          <span className={styles.agentExpandIcon}>{expanded ? "▾" : "▸"}</span>
        </div>
      </div>

      {expanded && (
        <div className={styles.agentCardBody}>
          <div className={styles.modelList}>
            {agent.models.map((m) => (
              <span key={m} className={styles.modelTag}>
                {m}
                <button
                  type="button"
                  className={styles.modelRemove}
                  onClick={() => onRemoveModel(m)}
                  title="移除模型"
                >
                  ×
                </button>
              </span>
            ))}
          </div>
          <div className={styles.addModelRow}>
            <input
              id={`add-model-${agent.id}`}
              name={`add-model-${agent.id}`}
              className={styles.addModelInput}
              placeholder="添加模型名称..."
              value={newModel}
              onChange={(e) => setNewModel(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleAdd();
              }}
              aria-label={`为 ${agent.name} 添加模型`}
            />
            <button type="button" className={styles.addModelBtn} onClick={handleAdd}>
              + 添加
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
