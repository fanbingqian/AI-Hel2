import { useEffect } from "react";
import { useUIStore } from "../../stores/uiStore";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";
import TabBar from "./TabBar";
import AiHelPage from "../aihel/AiHelPage";
import SettingsPage from "../settings/SettingsPage";
import AiWordPage from "../aiword/AiWordPage";
import styles from "./AppShell.module.css";

export default function AppShell() {
  const activePage = useUIStore((s) => s.activePage);
  const panelCollapsed = useUIStore((s) => s.panelCollapsed);
  const setPanelCollapsed = useUIStore((s) => s.setPanelCollapsed);

  // Initialize knowledge graph sync listeners once at app level
  useEffect(() => {
    const p = useKnowledgeStore.getState().setupEventListeners();
    return () => { p.then((fn) => fn()); };
  }, []);

  // Auto-expand when entering Settings or AI Word from collapsed mode
  useEffect(() => {
    if (activePage !== "aihel" && panelCollapsed) {
      setPanelCollapsed(false);
      const win = getCurrentWindow();
      win.setMaxSize(new LogicalSize(4000, 4000));
      win.setMinSize(new LogicalSize(900, 500));
      win.setSize(new LogicalSize(1232, 693));
    }
  }, [activePage]);

  return (
    <div className={styles.shell}>
      <TabBar compact={activePage === "aihel" && panelCollapsed} />
      <main className={styles.main}>
        {activePage === "aihel" && <AiHelPage />}
        {activePage === "aiword" && <AiWordPage />}
        {activePage === "settings" && <SettingsPage />}
      </main>
    </div>
  );
}
