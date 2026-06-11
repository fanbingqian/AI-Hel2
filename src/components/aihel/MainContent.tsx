import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { useUIStore, type MainContentMode } from "../../stores/uiStore";
import { ForceGraph2DWrapper } from "../sphere/ForceGraph2DWrapper";
import { FloatingMenu } from "../sphere/FloatingMenu";
import { GraphSettingsPanel } from "../sphere/GraphSettingsPanel";
import { LintPanel } from "../sphere/LintPanel";
import { EntityListPanel } from "../sphere/EntityListPanel";
import { CherryEditor } from "../knowledge/CherryEditor";
import { EntityBrowser } from "../knowledge/EntityBrowser";
import CanvasPage from "../canvas/CanvasPage";
import styles from "./AiHelPage.module.css";

interface Props {
  mode: MainContentMode;
  filePath: string | null;
  onFileOpen: (path: string, fileKind?: string) => void;
  onCloseFile: () => void;
}

export function MainContent({ mode, filePath, onFileOpen, onCloseFile }: Props) {
  const settingsOpen = useKnowledgeStore((s) => s.settingsOpen);
  const setSettingsOpen = useKnowledgeStore((s) => s.setSettingsOpen);
  const showLintPanel = useKnowledgeStore((s) => s.showLintPanel);
  const showEntityList = useKnowledgeStore((s) => s.showEntityList);

  // Graph mode (default)
  if (mode === "graph2d" || (!filePath && mode !== "editor" && mode !== "canvas" && mode !== "entity")) {
    return (
      <div className={styles.mainInner}>
        <ForceGraph2DWrapper />
        <FloatingMenu onOpenSettings={() => setSettingsOpen(true)} />
        {settingsOpen && <GraphSettingsPanel />}
        {showLintPanel && <LintPanel />}
        {showEntityList && <EntityListPanel />}
      </div>
    );
  }

  // Editor mode
  if (mode === "editor" && filePath) {
    return (
      <div className={styles.mainInner}>
        <CherryEditor filePath={filePath} onFileOpen={onFileOpen} />
        <button type="button" className={styles.closeFileBtn} onClick={onCloseFile} title="关闭文件">×</button>
      </div>
    );
  }

  // Canvas mode
  if (mode === "canvas" && filePath) {
    return (
      <div className={styles.mainInner}>
        <CanvasPage filePath={filePath} />
        <button type="button" className={styles.closeFileBtn} onClick={onCloseFile} title="关闭画板">×</button>
      </div>
    );
  }

  // Entity detail mode
  if (mode === "entity") {
    return (
      <div className={styles.mainInner}>
        <EntityBrowser detailOnly />
      </div>
    );
  }

  // Fallback
  return (
    <div className={styles.mainInner}>
      <ForceGraph2DWrapper />
      <FloatingMenu onOpenSettings={() => setSettingsOpen(true)} />
    </div>
  );
}
