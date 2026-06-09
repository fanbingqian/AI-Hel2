import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAgentRegistry, type AgentInfo } from "../../hooks/useAgentRegistry";
import styles from "./AgentSettings.module.css";

const MODEL_SUGGESTIONS = [
  "deepseek-chat", "deepseek-reasoner", "deepseek-v4-pro",
  "gpt-4o", "gpt-4o-mini", "gpt-4-turbo", "gpt-3.5-turbo",
  "claude-sonnet-4-6", "claude-opus-4-7", "claude-haiku-4-5",
  "llama-3.3-70b", "mixtral-8x7b", "gemma2-9b-it",
  "qwen-max", "qwen-plus", "glm-4-plus", "kimi-latest",
  "openclaw",
];

export function AgentSettings() {
  const { agents, loading, reDetect, addAgent, removeAgent, setEnabled, setDefault, refresh } = useAgentRegistry();
  const [showAdd, setShowAdd] = useState(false);
  const [detecting, setDetecting] = useState(false);
  const [detectResult, setDetectResult] = useState<string | null>(null);

  const handleReDetect = async () => {
    setDetecting(true);
    setDetectResult(null);
    try {
      await reDetect();
      setDetectResult("ok");
    } catch (e: any) {
      setDetectResult(typeof e === "string" ? e : e?.message || "检测失败");
    } finally {
      setDetecting(false);
      setTimeout(() => setDetectResult(null), 3000);
    }
  };

  if (loading) return <div className={styles.container}>加载中...</div>;

  return (
    <div className={styles.container}>
      <div className={styles.header}>
        <h2>Agent 管理</h2>
        <div className={styles.actions}>
          <button className={styles.btn} onClick={handleReDetect} disabled={detecting}>
            {detecting ? "检测中..." : "重新检测"}
          </button>
          <button className={styles.btnPrimary} onClick={() => setShowAdd(true)}>手动添加</button>
        </div>
      </div>
      {detectResult && (
        <div className={detectResult === "ok" ? styles.detectOk : styles.detectErr}>
          {detectResult === "ok" ? "检测完成" : detectResult}
        </div>
      )}

      <div className={styles.list}>
        {agents.map((agent) => (
          <AgentRow
            key={agent.id}
            agent={agent}
            onToggle={(enabled) => setEnabled(agent.id, enabled)}
            onRemove={() => { removeAgent(agent.id); refresh(); }}
            onSetDefault={() => setDefault(agent.id)}
            onRefresh={refresh}
          />
        ))}
      </div>

      {showAdd && (
        <AddAgentDialog
          onClose={() => setShowAdd(false)}
          onAdd={async (a) => { await addAgent(a.id, a.display_name, a.agent_type, a.config.base_url, a.config.api_key, a.config.models, a.config.vision_models || [], a.config.reasoning_models || []); setShowAdd(false); }}
        />
      )}
    </div>
  );
}

function AgentRow({ agent, onToggle, onRemove, onSetDefault, onRefresh }: {
  agent: AgentInfo;
  onToggle: (enabled: boolean) => void;
  onRemove: () => void;
  onSetDefault: () => void;
  onRefresh: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [baseUrl, setBaseUrl] = useState(agent.base_url);
  const [apiKey, setApiKey] = useState("");
  const [models, setModels] = useState<string[]>([...agent.models]);
  const [visionModels, setVisionModels] = useState<string[]>([...agent.vision_models]);
  const [reasoningModels, setReasoningModels] = useState<string[]>([...agent.reasoning_models]);
  const [newModel, setNewModel] = useState("");
  const [newVisionModel, setNewVisionModel] = useState("");
  const [newReasoningModel, setNewReasoningModel] = useState("");
  const [saving, setSaving] = useState(false);
  const [saveMsg, setSaveMsg] = useState<string | null>(null);

  const statusIcon = agent.status === "Running" ? "●" : agent.status === "Detected" ? "✓" : "○";
  const statusClass = agent.status === "Running" ? styles.statusRunning :
    agent.status === "Detected" ? styles.statusDetected : styles.statusOffline;
  const statusLabel = agent.status === "Running" ? "运行中" : agent.status === "Detected" ? "已检测" : "离线";
  const typeLabel = agent.agent_type === "hermes_builtin" ? "内置服务" : agent.agent_type === "openclaw" ? "OpenClaw" : "OpenAI 兼容";

  const handleAddModel = (m: string) => {
    if (m && !models.includes(m)) {
      setModels([...models, m]);
    }
    setNewModel("");
  };

  const handleRemoveModel = (m: string) => {
    setModels(models.filter((x) => x !== m));
  };

  const handleSave = async () => {
    setSaving(true);
    setSaveMsg(null);
    try {
      await invoke("update_agent_config", {
        id: agent.id,
        baseUrl: baseUrl || null,
        apiKey: apiKey ? apiKey : null,
        models,
        visionModels: visionModels,
        reasoningModels: reasoningModels,
      });
      setSaveMsg("ok");
      onRefresh();
    } catch (e: any) {
      setSaveMsg(typeof e === "string" ? e : e?.message || "保存失败");
    } finally {
      setSaving(false);
      setTimeout(() => setSaveMsg(null), 2500);
    }
  };

  return (
    <div className={`${styles.row} ${expanded ? styles.rowExpanded : ""}`}>
      <div className={styles.rowHeader} onClick={() => setExpanded(!expanded)}>
        <span className={`${styles.statusBadge} ${statusClass}`}>
          <span className={styles.statusDotInner}>{statusIcon}</span>
          {statusLabel}
        </span>
        <div className={styles.info}>
          <strong className={styles.agentName}>{agent.display_name}</strong>
          <span className={styles.meta}>
            {typeLabel}
            <span className={styles.metaSep}>|</span>
            {agent.models.length} 个模型
            <span className={styles.metaSep}>|</span>
            {agent.base_url ? agent.base_url.replace(/^https?:\/\//, "").replace(/\/v1$/, "") : "未配置"}
          </span>
        </div>
        <div className={styles.rowActions}>
          {agent.added_manually && (
            <button type="button" className={styles.btnSmallDanger} onClick={(e) => { e.stopPropagation(); onRemove(); }}>删除</button>
          )}
          <button type="button" className={styles.btnSmall} onClick={(e) => { e.stopPropagation(); onSetDefault(); }}>设为默认</button>
          <label className={styles.toggle} onClick={(e) => e.stopPropagation()}>
            <input type="checkbox" checked={agent.enabled} onChange={(e) => onToggle(e.target.checked)} />
            <span className={styles.slider}></span>
          </label>
        </div>
        <span className={styles.expandIcon}>{expanded ? "▾" : "▸"}</span>
      </div>

      {expanded && (
        <div className={styles.rowBody}>
          <div className={styles.configGrid}>
            <label className={styles.configField}>
              <span>Base URL</span>
              <input
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                placeholder="http://127.0.0.1:8080/v1"
              />
            </label>
            <label className={styles.configField}>
              <span>API Key</span>
              <input
                type="password"
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder="留空保持原有 key 不变"
              />
            </label>
          </div>

          <div className={styles.modelSection}>
            <span className={styles.modelLabel}>主大模型</span>
            <div className={styles.modelTags}>
              {models.map((m) => (
                <span key={m} className={styles.modelTag}>
                  {m}
                  <button type="button" className={styles.modelRemove} onClick={() => handleRemoveModel(m)}>&times;</button>
                </span>
              ))}
            </div>
            <div className={styles.addModelRow}>
              <input
                className={styles.addModelInput}
                value={newModel}
                onChange={(e) => setNewModel(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") handleAddModel(newModel); }}
                placeholder="输入或选择模型名称..."
                list="modelSuggestions"
              />
              <datalist id="modelSuggestions">
                {MODEL_SUGGESTIONS.map((m) => (<option key={m} value={m} />))}
              </datalist>
              <button type="button" className={styles.btnSmall} onClick={() => handleAddModel(newModel)}>+ 添加</button>
            </div>
          </div>

          <div className={styles.modelSection}>
            <span className={styles.modelLabel}>视觉大模型</span>
            <div className={styles.modelTags}>
              {visionModels.map((m) => (
                <span key={m} className={styles.modelTag}>
                  {m}
                  <button type="button" className={styles.modelRemove} onClick={() => setVisionModels(visionModels.filter((x) => x !== m))}>&times;</button>
                </span>
              ))}
            </div>
            <div className={styles.addModelRow}>
              <input
                className={styles.addModelInput}
                value={newVisionModel}
                onChange={(e) => setNewVisionModel(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") { const m = newVisionModel; if (m && !visionModels.includes(m)) { setVisionModels([...visionModels, m]); } setNewVisionModel(""); } }}
                placeholder="输入视觉模型名称..."
                list="visionModelSuggestions"
              />
              <datalist id="visionModelSuggestions">
                {MODEL_SUGGESTIONS.map((m) => (<option key={m} value={m} />))}
              </datalist>
              <button type="button" className={styles.btnSmall} onClick={() => { const m = newVisionModel; if (m && !visionModels.includes(m)) { setVisionModels([...visionModels, m]); } setNewVisionModel(""); }}>+ 添加</button>
            </div>
          </div>

          <div className={styles.modelSection}>
            <span className={styles.modelLabel}>推理大模型</span>
            <div className={styles.modelTags}>
              {reasoningModels.map((m) => (
                <span key={m} className={styles.modelTag}>
                  {m}
                  <button type="button" className={styles.modelRemove} onClick={() => setReasoningModels(reasoningModels.filter((x) => x !== m))}>&times;</button>
                </span>
              ))}
            </div>
            <div className={styles.addModelRow}>
              <input
                className={styles.addModelInput}
                value={newReasoningModel}
                onChange={(e) => setNewReasoningModel(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") { const m = newReasoningModel; if (m && !reasoningModels.includes(m)) { setReasoningModels([...reasoningModels, m]); } setNewReasoningModel(""); } }}
                placeholder="输入推理模型名称..."
                list="reasoningModelSuggestions"
              />
              <datalist id="reasoningModelSuggestions">
                {MODEL_SUGGESTIONS.map((m) => (<option key={m} value={m} />))}
              </datalist>
              <button type="button" className={styles.btnSmall} onClick={() => { const m = newReasoningModel; if (m && !reasoningModels.includes(m)) { setReasoningModels([...reasoningModels, m]); } setNewReasoningModel(""); }}>+ 添加</button>
            </div>
          </div>

          <div className={styles.saveRow}>
            {saveMsg && (
              <span className={saveMsg === "ok" ? styles.saveOk : styles.saveErr}>
                {saveMsg === "ok" ? "已保存" : saveMsg}
              </span>
            )}
            <button className={styles.btnPrimary} onClick={handleSave} disabled={saving}>
              {saving ? "保存中..." : "保存配置"}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

const PROVIDER_PRESETS = [
  { label: "自定义", id: "", name: "", url: "", models: [] as string[], visionModels: [] as string[], reasoningModels: [] as string[], type: "openai_compatible" },
  { label: "DeepSeek", id: "deepseek", name: "DeepSeek", url: "https://api.deepseek.com/v1", models: ["deepseek-chat"], visionModels: [], reasoningModels: ["deepseek-reasoner"], type: "openai_compatible" },
  { label: "OpenAI", id: "openai", name: "OpenAI", url: "https://api.openai.com/v1", models: ["gpt-4o", "gpt-4o-mini"], visionModels: ["gpt-4o"], reasoningModels: ["o3-mini"], type: "openai_compatible" },
  { label: "Anthropic", id: "anthropic", name: "Anthropic", url: "https://api.anthropic.com/v1", models: ["claude-sonnet-4-6", "claude-opus-4-7"], visionModels: ["claude-sonnet-4-6"], reasoningModels: [], type: "openai_compatible" },
  { label: "Groq", id: "groq", name: "Groq", url: "https://api.groq.com/openai/v1", models: ["llama-3.3-70b", "mixtral-8x7b"], visionModels: [], reasoningModels: [], type: "openai_compatible" },
  { label: "Together AI", id: "together", name: "Together AI", url: "https://api.together.xyz/v1", models: ["meta-llama/Llama-3.3-70B-Instruct-Turbo"], visionModels: [], reasoningModels: [], type: "openai_compatible" },
  { label: "Ollama (本地)", id: "ollama", name: "Ollama", url: "http://localhost:11434/v1", models: [], visionModels: [], reasoningModels: [], type: "openai_compatible" },
  { label: "LM Studio (本地)", id: "lmstudio", name: "LM Studio", url: "http://localhost:1234/v1", models: [], visionModels: [], reasoningModels: [], type: "openai_compatible" },
];

function AddAgentDialog({ onClose, onAdd }: {
  onClose: () => void;
  onAdd: (a: any) => void;
}) {
  const [preset, setPreset] = useState("");
  const [id, setId] = useState("");
  const [name, setName] = useState("");
  const [type, setType] = useState("openai_compatible");
  const [url, setUrl] = useState("");
  const [key, setKey] = useState("");
  const [models, setModels] = useState<string[]>([]);
  const [visionModels, setVisionModels] = useState<string[]>([]);
  const [reasoningModels, setReasoningModels] = useState<string[]>([]);
  const [newModel, setNewModel] = useState("");
  const [newVisionModel, setNewVisionModel] = useState("");
  const [newReasoningModel, setNewReasoningModel] = useState("");

  const isValid = id.trim() && name.trim() && url.trim();

  const handlePresetChange = (presetId: string) => {
    setPreset(presetId);
    const p = PROVIDER_PRESETS.find((x) => x.id === presetId);
    if (p) {
      setId(p.id);
      setName(p.name);
      setType(p.type);
      setUrl(p.url);
      setModels([...p.models]);
      setVisionModels([...p.visionModels]);
      setReasoningModels([...p.reasoningModels]);
    }
  };

  const handleAddModel = (m: string) => {
    if (m && !models.includes(m)) {
      setModels([...models, m]);
    }
    setNewModel("");
  };

  const handleRemoveModel = (m: string) => {
    setModels(models.filter((x) => x !== m));
  };

  const handleSubmit = () => {
    if (!isValid) return;
    onAdd({
      id, display_name: name, agent_type: type,
      config: {
        base_url: url,
        api_key: key || null,
        models,
        vision_models: visionModels,
        reasoning_models: reasoningModels,
      },
    });
  };

  return (
    <div className={styles.overlay}>
      <div className={styles.dialog}>
        <h3>添加 Agent</h3>

        <label>模型提供商:
          <select value={preset} onChange={(e) => handlePresetChange(e.target.value)}>
            <option value="">-- 选择提供商（自动填充 Base URL 和模型） --</option>
            {PROVIDER_PRESETS.map((p) => (
              <option key={p.id} value={p.id}>{p.label}{p.url ? ` — ${p.url}` : ""}</option>
            ))}
          </select>
        </label>

        <div className={styles.dialogDivider} />

        <div className={styles.configGrid}>
          <label className={styles.configField}>
            <span>标识 ID</span>
            <input value={id} onChange={(e) => setId(e.target.value)} placeholder="my-agent" />
          </label>
          <label className={styles.configField}>
            <span>显示名称</span>
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="My Agent" />
          </label>
        </div>

        <label>Base URL: <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="http://127.0.0.1:8080/v1" /></label>

        <label>API Key:
          <input type="password" value={key} onChange={(e) => setKey(e.target.value)} placeholder="sk-..." />
          <span className={styles.dialogHint}>仅需填写 API Key 即可使用已选提供商</span>
        </label>

        <div className={styles.dialogModelSection}>
          <span>主大模型</span>
          <div className={styles.modelTags}>
            {models.map((m) => (
              <span key={m} className={styles.modelTag}>
                {m}
                <button type="button" className={styles.modelRemove} onClick={() => handleRemoveModel(m)}>&times;</button>
              </span>
            ))}
          </div>
          <div className={styles.addModelRow}>
            <input
              className={styles.addModelInput}
              value={newModel}
              onChange={(e) => setNewModel(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") handleAddModel(newModel); }}
              placeholder="输入或选择模型名称..."
              list="dialogModelSuggestions"
            />
            <datalist id="dialogModelSuggestions">
              {MODEL_SUGGESTIONS.map((m) => (<option key={m} value={m} />))}
            </datalist>
            <button type="button" className={styles.btnSmall} onClick={() => handleAddModel(newModel)}>+ 添加</button>
          </div>
        </div>

        <div className={styles.dialogModelSection}>
          <span>视觉大模型</span>
          <div className={styles.modelTags}>
            {visionModels.map((m) => (
              <span key={m} className={styles.modelTag}>
                {m}
                <button type="button" className={styles.modelRemove} onClick={() => setVisionModels(visionModels.filter((x) => x !== m))}>&times;</button>
              </span>
            ))}
          </div>
          <div className={styles.addModelRow}>
            <input
              className={styles.addModelInput}
              value={newVisionModel}
              onChange={(e) => setNewVisionModel(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") { const m = newVisionModel; if (m && !visionModels.includes(m)) { setVisionModels([...visionModels, m]); } setNewVisionModel(""); } }}
              placeholder="输入视觉模型名称..."
              list="dialogVisionModelSuggestions"
            />
            <datalist id="dialogVisionModelSuggestions">
              {MODEL_SUGGESTIONS.map((m) => (<option key={m} value={m} />))}
            </datalist>
            <button type="button" className={styles.btnSmall} onClick={() => { const m = newVisionModel; if (m && !visionModels.includes(m)) { setVisionModels([...visionModels, m]); } setNewVisionModel(""); }}>+ 添加</button>
          </div>
        </div>

        <div className={styles.dialogModelSection}>
          <span>推理大模型</span>
          <div className={styles.modelTags}>
            {reasoningModels.map((m) => (
              <span key={m} className={styles.modelTag}>
                {m}
                <button type="button" className={styles.modelRemove} onClick={() => setReasoningModels(reasoningModels.filter((x) => x !== m))}>&times;</button>
              </span>
            ))}
          </div>
          <div className={styles.addModelRow}>
            <input
              className={styles.addModelInput}
              value={newReasoningModel}
              onChange={(e) => setNewReasoningModel(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") { const m = newReasoningModel; if (m && !reasoningModels.includes(m)) { setReasoningModels([...reasoningModels, m]); } setNewReasoningModel(""); } }}
              placeholder="输入推理模型名称..."
              list="dialogReasoningModelSuggestions"
            />
            <datalist id="dialogReasoningModelSuggestions">
              {MODEL_SUGGESTIONS.map((m) => (<option key={m} value={m} />))}
            </datalist>
            <button type="button" className={styles.btnSmall} onClick={() => { const m = newReasoningModel; if (m && !reasoningModels.includes(m)) { setReasoningModels([...reasoningModels, m]); } setNewReasoningModel(""); }}>+ 添加</button>
          </div>
        </div>

        <div className={styles.dialogActions}>
          <button onClick={onClose}>取消</button>
          <button className={styles.btnPrimary} onClick={handleSubmit} disabled={!isValid}>添加</button>
        </div>
      </div>
    </div>
  );
}
