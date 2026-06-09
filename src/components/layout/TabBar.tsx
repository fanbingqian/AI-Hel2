import { useUIStore } from "../../stores/uiStore";
import { useSettingsStore } from "../../stores/settingsStore";
import { useAgentRegistry } from "../../hooks/useAgentRegistry";
import type { PageId } from "../../types";
import styles from "./TabBar.module.css";

const tabs: { id: PageId; label: string }[] = [
  { id: "sphere", label: "对话球体" },
  { id: "knowledge", label: "知识编辑" },
  { id: "canvas", label: "画板" },
  { id: "aiword", label: "AI Word" },
];

export default function TabBar() {
  const activePage = useUIStore((s) => s.activePage);
  const setActivePage = useUIStore((s) => s.setActivePage);
  const user = useSettingsStore((s) => s.user);
  const { agents, activeAgentId, setActive } = useAgentRegistry();

  return (
    <header className={styles.tabBar}>
      <div className={styles.tabs}>
        {tabs.map((tab) => (
          <button
            key={tab.id}
            className={`${styles.tab} ${activePage === tab.id ? styles.active : ""}`}
            onClick={() => setActivePage(tab.id)}
          >
            {tab.label}
          </button>
        ))}
      </div>

      <div className={styles.right}>
        <select
          className={styles.tabAgentSelect}
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
        {user && (
          <div className={styles.agentStatus} title="已登录">
            <span className={styles.avatar}>{user.avatarLetter}</span>
            <span className={styles.statusText}>{user.name}</span>
          </div>
        )}
        <button className={`${styles.settingsBtn} ${activePage === "settings" ? styles.active : ""}`} title="设置" onClick={() => setActivePage("settings")}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
          </svg>
        </button>
      </div>
    </header>
  );
}
