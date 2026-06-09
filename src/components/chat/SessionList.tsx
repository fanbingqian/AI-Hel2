import { useEffect, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Plus, X } from "lucide-react";
import { useChatStore } from "../../stores/chatStore";
import { useUIStore } from "../../stores/uiStore";
import styles from "./SessionList.module.css";

interface Session {
  id: string;
  title: string;
  model: string;
  created_at: string;
  updated_at: string;
  message_count: number;
}

export function SessionList() {
  const storeSessions = useChatStore((s) => s.sessions);
  const [search, setSearch] = useState("");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editTitle, setEditTitle] = useState("");
  const activeSessionId = useChatStore((s) => s.sessionId);
  const setSessionId = useChatStore((s) => s.setSessionId);
  const clearMessages = useChatStore((s) => s.clearMessages);
  const toggleSessionList = useUIStore((s) => s.toggleSessionList);

  // Convert store sessions to local format
  const sessions = storeSessions.map((s: any) => ({
    id: s.id,
    title: s.title,
    model: s.model || "",
    created_at: s.createdAt ? new Date(s.createdAt).toISOString() : (s.created_at || ""),
    updated_at: s.updatedAt ? new Date(s.updatedAt).toISOString() : (s.updated_at || ""),
    message_count: s.messageCount ?? s.message_count ?? 0,
  }));

  const loadSessions = useCallback(() => {
    useChatStore.getState().loadSessions();
  }, []);

  useEffect(() => {
    loadSessions();
  }, [loadSessions]);

  const handleNew = () => {
    const id = Date.now().toString(36);
    setSessionId(id);
    clearMessages();
    // Save session to DB immediately so it appears in the list
    invoke("upsert_session", {
      id,
      title: "新会话",
      model: "deepseek-v4-pro",
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    }).then(() => loadSessions()).catch(() => {});
    toggleSessionList();
  };

  const handleSelect = (id: string) => {
    if (editingId) return;
    setSessionId(id);
    clearMessages();
    toggleSessionList();
  };

  const handleDelete = (e: React.MouseEvent, id: string) => {
    e.stopPropagation();
    invoke("delete_session", { sessionId: id })
      .then(() => {
        if (activeSessionId === id) {
          clearMessages();
        }
        loadSessions();
      })
      .catch(console.error);
  };

  const handleStartRename = (e: React.MouseEvent, s: Session) => {
    e.stopPropagation();
    setEditingId(s.id);
    setEditTitle(s.title || "新会话");
  };

  const handleRenameSubmit = (id: string) => {
    if (editTitle.trim()) {
      invoke("rename_session", { sessionId: id, title: editTitle.trim() })
        .then(() => loadSessions())
        .catch(console.error);
    }
    setEditingId(null);
    setEditTitle("");
  };

  const handleSearch = useCallback((q: string) => {
    setSearch(q);
    if (q.trim()) {
      invoke<Array<{ session_id: string; session_title: string; matched_line: string }>>(
        "search_sessions", { query: q }
      )
        .then((results) => {
          const ids = new Set(results.map((r) => r.session_id));
          invoke<Session[]>("list_sessions").then((all) => {
            useChatStore.setState({
              sessions: all.filter((s) => ids.has(s.id)).map((s) => ({
                id: s.id,
                title: s.title,
                model: s.model,
                messageCount: s.message_count,
                createdAt: new Date(s.created_at).getTime(),
                updatedAt: new Date(s.updated_at).getTime(),
              })),
            });
          }).catch(() => {});
        })
        .catch(() => loadSessions());
    } else {
      loadSessions();
    }
  }, [loadSessions]);

  const filtered = sessions.filter((s) =>
    !search || s.title.toLowerCase().includes(search.toLowerCase())
  );

  return (
    <>
      <div className={styles.backdrop} onClick={toggleSessionList} />
      <div className={styles.overlay}>
        <div className={styles.header}>
          <span className={styles.headerTitle}>会话列表</span>
          <div className={styles.headerActions}>
            <button className={styles.newBtn} onClick={handleNew} title="新建会话">
              <Plus size={16} />
            </button>
            <button
              className={styles.closeBtn}
              onClick={toggleSessionList}
              title="关闭"
            >
              <X size={16} />
            </button>
          </div>
        </div>
        <div className={styles.searchBox}>
          <input
            id="session-search"
            name="session-search"
            className={styles.searchInput}
            placeholder="搜索会话..."
            value={search}
            onChange={(e) => {
              setSearch(e.target.value);
              if (!e.target.value.trim()) loadSessions();
            }}
            onKeyDown={(e) => { if (e.key === "Enter") handleSearch(search); }}
            aria-label="搜索会话"
          />
        </div>
        <div className={styles.list}>
          {filtered.length === 0 && (
            <div className={styles.empty}>
              {search ? "无匹配会话" : "暂无会话"}
            </div>
          )}
          {filtered.map((s) => (
            <div
              key={s.id}
              className={`${styles.item} ${s.id === activeSessionId ? styles.active : ""}`}
              onClick={() => handleSelect(s.id)}
              onDoubleClick={(e) => handleStartRename(e, s)}
            >
              {editingId === s.id ? (
                <input
                  id={`rename-session-${s.id}`}
                  name={`rename-session-${s.id}`}
                  className={styles.inlineEdit}
                  value={editTitle}
                  onChange={(e) => setEditTitle(e.target.value)}
                  onBlur={() => handleRenameSubmit(s.id)}
                  aria-label="重命名会话"
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleRenameSubmit(s.id);
                    if (e.key === "Escape") setEditingId(null);
                  }}
                  autoFocus
                  onClick={(e) => e.stopPropagation()}
                />
              ) : (
                <>
                  <div className={styles.itemTitle}>{s.title || "新会话"}</div>
                  <div className={styles.itemMeta}>
                    {s.message_count} 条消息 · {s.updated_at?.slice(0, 10)}
                  </div>
                </>
              )}
              {editingId !== s.id && (
                <button
                  className={styles.deleteBtn}
                  onClick={(e) => handleDelete(e, s.id)}
                  title="删除会话"
                >
                  <X size={12} />
                </button>
              )}
            </div>
          ))}
        </div>
      </div>
    </>
  );
}
