import { useAgentRegistry } from "../../hooks/useAgentRegistry";
import styles from "./AgentSwitcher.module.css";

export function AgentSwitcher() {
  const { agents, activeAgentId, setActive } = useAgentRegistry();
  const activeAgent = agents.find((a) => a.id === activeAgentId);
  const connected = activeAgent?.status === "Running";

  return (
    <div className={styles.switcher}>
      <select
        className={styles.select}
        value={activeAgentId}
        onChange={(e) => setActive(e.target.value)}
        title="切换 Agent"
      >
        {agents.filter(a => a.enabled).map((a) => (
          <option key={a.id} value={a.id}>
            {a.display_name}
          </option>
        ))}
      </select>
      <span
        className={styles.connDot}
        data-connected={connected ? "1" : "0"}
        title={connected ? "已连接" : "未连接"}
      />
    </div>
  );
}
