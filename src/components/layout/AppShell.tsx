import { useState, useEffect } from "react";
import { useUIStore } from "../../stores/uiStore";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import TabBar from "./TabBar";
import SphereChatView from "../sphere/SphereChatView";
import KnowledgeEditor from "../knowledge/KnowledgeEditor";
import CanvasPage from "../canvas/CanvasPage";
import SettingsPage from "../settings/SettingsPage";
import AiWordPage from "../aiword/AiWordPage";
import styles from "./AppShell.module.css";

export default function AppShell() {
  const activePage = useUIStore((s) => s.activePage);
  const setActivePage = useUIStore((s) => s.setActivePage);
  const [canvasPath, setCanvasPath] = useState<string | null>(null);

  // Initialize knowledge graph sync listeners once at app level
  useEffect(() => {
    const p = useKnowledgeStore.getState().setupEventListeners();
    return () => { p.then((fn) => fn()); };
  }, []);

  const handleCanvasOpen = (path: string) => {
    setCanvasPath(path);
    setActivePage("canvas");
  };

  return (
    <div className={styles.shell}>
      <TabBar />
      <main className={styles.main}>
        {activePage === "sphere" && <SphereChatView />}
        {activePage === "knowledge" && <KnowledgeEditor onCanvasOpen={handleCanvasOpen} />}
        {activePage === "canvas" && <CanvasPage filePath={canvasPath} />}
        {activePage === "settings" && <SettingsPage />}
        {activePage === "aiword" && <AiWordPage />}
      </main>
    </div>
  );
}
