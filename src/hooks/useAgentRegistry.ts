import { useEffect, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useChatStore } from "../stores/chatStore";

export interface AgentInfo {
  id: string;
  display_name: string;
  agent_type: string;
  enabled: boolean;
  models: string[];
  vision_models: string[];
  reasoning_models: string[];
  healthy: boolean;
  detected: boolean;
  added_manually: boolean;
  status: "Running" | "Detected" | "Offline";
  base_url: string;
}

export function useAgentRegistry() {
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [activeAgentId, setActiveAgentId] = useState<string>(() => {
    return localStorage.getItem("activeAgentId") || "hermes-builtin";
  });
  const [loading, setLoading] = useState(true);

  const fetchAgents = useCallback(async () => {
    try {
      const list = await invoke<AgentInfo[]>("list_agents");
      setAgents(list);
    } catch (e) {
      console.error("[useAgentRegistry] list_agents failed:", e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchAgents();
    const u = listen("agents:updated", () => {
      fetchAgents();
    });
    return () => { u.then((fn) => fn()); };
  }, [fetchAgents]);

  const setActive = useCallback((id: string) => {
    setActiveAgentId(id);
    localStorage.setItem("activeAgentId", id);
    const agent = agents.find((a) => a.id === id);
    if (agent && agent.models.length > 0 && !localStorage.getItem(`agentModel_${id}`)) {
      localStorage.setItem(`agentModel_${id}`, agent.models[0]);
    }
    useChatStore.getState().setAgentId(id);
    // 通知 AI Word (Claw3D) iframe：Agent 切换
    const iframes = document.querySelectorAll('iframe[title="AI Word"]');
    iframes.forEach((iframe) => {
      try {
        (iframe as HTMLIFrameElement).contentWindow?.postMessage(
          { type: "agent-switch", agentId: id, agentName: agent?.display_name || id },
          "*"
        );
      } catch {}
    });
  }, [agents]);

  const activeAgent = agents.find((a) => a.id === activeAgentId) || null;

  const activeModels = activeAgent?.models || [];

  const addAgent = useCallback(async (
    id: string, display_name: string, agent_type: string,
    base_url: string, api_key: string | null,
    models: string[], vision_models: string[], reasoning_models: string[],
  ) => {
    await invoke("add_agent", { id, displayName: display_name, agentType: agent_type, baseUrl: base_url, apiKey: api_key, models, visionModels: vision_models, reasoningModels: reasoning_models });
    await fetchAgents();
  }, [fetchAgents]);

  const removeAgent = useCallback(async (id: string) => {
    await invoke("remove_agent", { id });
    await fetchAgents();
  }, [fetchAgents]);

  const setEnabled = useCallback(async (id: string, enabled: boolean) => {
    await invoke("set_agent_enabled", { id, enabled });
    await fetchAgents();
  }, [fetchAgents]);

  const setDefault = useCallback(async (id: string) => {
    await invoke("set_default_agent", { id });
    setActive(id);
    await fetchAgents();
  }, [fetchAgents, setActive]);

  const reDetect = useCallback(async () => {
    await invoke("re_detect_agents");
    await fetchAgents();
  }, [fetchAgents]);

  return {
    agents, activeAgentId, activeAgent, activeModels, loading,
    setActive, addAgent, removeAgent, setEnabled, setDefault, reDetect,
    refresh: fetchAgents,
  };
}
