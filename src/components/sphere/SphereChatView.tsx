import { useCallback, useEffect } from "react";
import { useUIStore } from "../../stores/uiStore";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { PanelResizer } from "../layout/PanelResizer";
import { ForceGraph2DWrapper } from "./ForceGraph2DWrapper";

import { FloatingMenu } from "./FloatingMenu";
import { GraphSettingsPanel } from "./GraphSettingsPanel";
import { LintPanel } from "./LintPanel";
import { EntityListPanel } from "./EntityListPanel";
import { ChatPanel } from "../chat/ChatPanel";
import { SessionList } from "../chat/SessionList";
import styles from "./SphereChatView.module.css";

export default function SphereChatView() {
  const chatPanelWidth = useUIStore((s) => s.chatPanelWidth);
  const setChatPanelWidth = useUIStore((s) => s.setChatPanelWidth);
  const sessionListOpen = useUIStore((s) => s.sessionListExpanded);

  const settingsOpen = useKnowledgeStore((s) => s.settingsOpen);
  const setSettingsOpen = useKnowledgeStore((s) => s.setSettingsOpen);
  const showLintPanel = useKnowledgeStore((s) => s.showLintPanel);
  const showEntityList = useKnowledgeStore((s) => s.showEntityList);
  const loadGraphSettings = useKnowledgeStore((s) => s.loadGraphSettings);
  const fetchGraphData = useKnowledgeStore((s) => s.fetchGraphData);
  const loadInferences = useKnowledgeStore((s) => s.loadInferences);

  // Initial data load
  useEffect(() => { fetchGraphData(); loadInferences(); }, [fetchGraphData, loadInferences]);
  useEffect(() => { loadGraphSettings(); }, [loadGraphSettings]);

  const handleChatResize = useCallback(
    (delta: number) => {
      setChatPanelWidth(chatPanelWidth - delta);
    },
    [chatPanelWidth, setChatPanelWidth],
  );

  return (
    <div className={styles.container}>
      <ForceGraph2DWrapper />
      <FloatingMenu onOpenSettings={() => setSettingsOpen(true)} />
      {settingsOpen && <GraphSettingsPanel />}
      {showLintPanel && <LintPanel />}
      {showEntityList && <EntityListPanel />}
      <PanelResizer onResize={handleChatResize} />
      <div className={styles.chatColumn}>
        <ChatPanel />
        {sessionListOpen && <SessionList />}
      </div>
    </div>
  );
}
