import { useEffect, useState, useCallback, useRef } from "react";
import { ChevronRight, ChevronDown, File, Folder, FolderOpen, Plus, Trash2, FolderPlus, Pencil, MoveRight, ExternalLink, Upload, FilePlus, Palette } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import * as wikiService from "../../services/wiki";
import type { FileNode } from "../../types/wiki";
import styles from "./DocTree.module.css";

interface Props {
  onFileOpen: (path: string, fileKind?: string) => void;
  onCanvasOpen?: (path: string) => void;
  onUploadClick?: () => void;
  onRename?: (oldPath: string, newPath: string) => void;
  refreshKey?: number;
}

interface ContextMenuState {
  x: number;
  y: number;
  node: FileNode;
}

interface MoveModalState {
  node: FileNode;
  folders: string[];
}

const ICON_MAP: Record<string, string> = {
  canvas: "🎨",
  png: "🖼",
  jpg: "🖼",
  jpeg: "🖼",
  gif: "🖼",
  svg: "🖼",
  pdf: "📄",
  md: "📝",
};

function getIcon(name: string, isFolder: boolean): string {
  if (isFolder) return "";
  const ext = name.split(".").pop()?.toLowerCase() || "";
  return ICON_MAP[ext] || "📄";
}

function filterTree(nodes: FileNode[], query: string): FileNode[] {
  const q = query.toLowerCase();
  const result: FileNode[] = [];
  for (const node of nodes) {
    const nameMatch = node.name.toLowerCase().includes(q);
    const titleMatch = node.title?.toLowerCase().includes(q);
    const selfMatch = nameMatch || titleMatch;
    if (node.type === "folder" && node.children) {
      const filteredChildren = filterTree(node.children, query);
      if (selfMatch || filteredChildren.length > 0) {
        result.push({ ...node, children: filteredChildren.length > 0 ? filteredChildren : node.children });
      }
    } else if (selfMatch) {
      result.push(node);
    }
  }
  return result;
}

function getAllFolderPaths(nodes: FileNode[]): string[] {
  const paths: string[] = [""];
  function walk(list: FileNode[]) {
    for (const n of list) {
      if (n.type === "folder") {
        paths.push(n.path);
        if (n.children) walk(n.children);
      }
    }
  }
  walk(nodes);
  return paths;
}

function TreeNode({ node, depth, selectedPath, onSelect, onRefresh, onContextMenu, forceExpand }: {
  node: FileNode;
  depth: number;
  selectedPath: string | null;
  onSelect: (path: string, fileKind?: string) => void;
  onRefresh: () => void;
  onContextMenu: (e: React.MouseEvent, node: FileNode) => void;
  forceExpand?: boolean;
}) {
  const [expanded, setExpanded] = useState(depth < 1 || forceExpand);
  useEffect(() => {
    if (forceExpand) setExpanded(true);
  }, [forceExpand]);
  const isExpanded = forceExpand || expanded;
  const [dragOver, setDragOver] = useState(false);
  const isFolder = node.type === "folder";
  const isSelected = node.path === selectedPath;

  const handleDelete = async (e: React.MouseEvent) => {
    e.stopPropagation();
    await wikiService.deleteWikiItem(node.path);
    onRefresh();
  };

  const handleCreateFile = async (e: React.MouseEvent) => {
    e.stopPropagation();
    const raw = prompt("文件名:");
    if (raw) {
      try {
        const name = /\.\w+$/.test(raw) ? raw : `${raw}.md`;
        await wikiService.createWikiItem(node.path, name, "file");
        setExpanded(true);
        onRefresh();
      } catch (e) {
        alert(`创建失败: ${e}`);
      }
    }
  };

  const handleCreateFolder = async (e: React.MouseEvent) => {
    e.stopPropagation();
    const name = prompt("文件夹名:");
    if (name) {
      try {
        await wikiService.createWikiItem(node.path, name, "folder");
        setExpanded(true);
        onRefresh();
      } catch (e) {
        alert(`创建失败: ${e}`);
      }
    }
  };

  const handleDragStart = (e: React.DragEvent) => {
    e.stopPropagation();
    e.dataTransfer.setData("text/plain", node.path);
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.dropEffect = "move";
  };

  const handleDragOver = (e: React.DragEvent) => {
    if (!isFolder) return;
    e.preventDefault();
    e.stopPropagation();
    e.dataTransfer.dropEffect = "move";
    setDragOver(true);
  };

  const handleDragLeave = (e: React.DragEvent) => {
    e.stopPropagation();
    setDragOver(false);
  };

  const handleDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOver(false);
    const srcPath = e.dataTransfer.getData("text/plain");
    if (!srcPath || srcPath === node.path) return;
    const name = srcPath.split("/").pop() || "";
    const destPath = node.path ? `${node.path}/${name}` : name;
    try {
      await wikiService.moveWikiItem(srcPath, destPath);
      onRefresh();
    } catch (err) {
      alert(`移动失败: ${err}`);
    }
  };

  const icon = isFolder
    ? (isExpanded ? <FolderOpen size={12} /> : <Folder size={12} />)
    : null;

  const emoji = getIcon(node.name, isFolder);

  return (
    <div>
      <div
        className={`${styles.treeItem} ${isSelected ? styles.active : ""} ${dragOver ? styles.dragOver : ""}`}
        style={{ paddingLeft: 8 + depth * 16 }}
        draggable={true}
        onDragStart={handleDragStart}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
        onClick={() => {
          if (isFolder && !forceExpand) setExpanded(!expanded);
          onSelect(node.path, node.file_kind ?? undefined);
        }}
        onContextMenu={(e) => onContextMenu(e, node)}
      >
        <span className={styles.chevron}>
          {isFolder ? (isExpanded
            ? <ChevronDown size={10} />
            : <ChevronRight size={10} />) : null}
        </span>
        <span className={styles.icon}>
          {emoji ? <span style={{fontSize:11}}>{emoji}</span> : icon}
        </span>
        <span className={styles.itemTitle}>
          {node.title || node.name}
          {node.namespace && <span className={styles.namespaceTag}>{node.namespace}</span>}
        </span>
        {isFolder && (
          <span className={styles.actions}>
            <button className={styles.actionBtn} onClick={handleCreateFile} title="新建文件">
              <Plus size={10} />
            </button>
            <button className={styles.actionBtn} onClick={handleCreateFolder} title="新建文件夹">
              <FolderPlus size={10} />
            </button>
            <button className={styles.actionBtn} onClick={handleDelete} title="删除">
              <Trash2 size={10} />
            </button>
          </span>
        )}
      </div>
      {isFolder && isExpanded && node.children?.map((child) => (
        <TreeNode
          key={child.path}
          node={child}
          depth={depth + 1}
          selectedPath={selectedPath}
          onSelect={onSelect}
          onRefresh={onRefresh}
          onContextMenu={onContextMenu}
          forceExpand={forceExpand}
        />
      ))}
    </div>
  );
}

export function DocTree({ onFileOpen, onCanvasOpen, onUploadClick, onRename, refreshKey }: Props) {
  const [tree, setTree] = useState<FileNode[]>([]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [moveModal, setMoveModal] = useState<MoveModalState | null>(null);
  const treeRef = useRef<HTMLDivElement>(null);

  const loadTree = useCallback(() => {
    wikiService.getWikiFileTree().then(setTree).catch(console.error);
  }, []);

  useEffect(() => {
    loadTree();
  }, [loadTree]);

  // Reload tree when parent signals a save occurred (e.g. CherryEditor saved)
  useEffect(() => {
    if (refreshKey !== undefined) loadTree();
  }, [refreshKey, loadTree]);

  // Listen for file system changes from backend watcher
  useEffect(() => {
    const unlisten = listen("wiki:files-changed", () => {
      loadTree();
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [loadTree]);

  useEffect(() => {
    const close = () => setContextMenu(null);
    document.addEventListener("click", close);
    return () => document.removeEventListener("click", close);
  }, []);

  const handleSelect = (path: string, fileKind?: string) => {
    setSelectedPath(path);
    if (path.endsWith(".canvas") && onCanvasOpen) {
      onCanvasOpen(path);
    } else {
      onFileOpen(path, fileKind);
    }
  };

  const handleContextMenu = (e: React.MouseEvent, node: FileNode) => {
    e.preventDefault();
    e.stopPropagation();
    const treeEl = treeRef.current;
    if (treeEl) {
      const rect = treeEl.getBoundingClientRect();
      setContextMenu({ x: e.clientX - rect.left, y: e.clientY - rect.top, node });
    } else {
      setContextMenu({ x: e.clientX, y: e.clientY, node });
    }
  };

  const handleRename = async () => {
    if (!contextMenu) return;
    const oldName = contextMenu.node.name;
    const newName = prompt("新名称:", oldName);
    if (newName && newName !== oldName) {
      try {
        // Preserve original extension if new name lacks one
        let finalName = newName;
        const oldExt = oldName.includes(".") ? oldName.split(".").pop() : "";
        const newHasExt = newName.includes(".");
        if (oldExt && !newHasExt) {
          finalName = `${newName}.${oldExt}`;
        }
        await wikiService.renameWikiItem(contextMenu.node.path, finalName);
        // Compute new path so parent can update open file
        const parentPath = contextMenu.node.path.split("/").slice(0, -1).join("/");
        const newPath = parentPath ? `${parentPath}/${finalName}` : finalName;
        onRename?.(contextMenu.node.path, newPath);
        setContextMenu(null);
        loadTree();
      } catch (e) {
        alert(`重命名失败: ${e}`);
      }
    }
  };

  const handleMoveClick = () => {
    if (!contextMenu) return;
    const folders = getAllFolderPaths(tree);
    setMoveModal({ node: contextMenu.node, folders });
    setContextMenu(null);
  };

  const handleMoveConfirm = async (destPath: string) => {
    if (!moveModal) return;
    const name = moveModal.node.name;
    const newPath = destPath ? `${destPath}/${name}` : name;
    try {
      await wikiService.moveWikiItem(moveModal.node.path, newPath);
      setMoveModal(null);
      loadTree();
    } catch (e) {
      alert(`移动失败: ${e}`);
    }
  };

  const handleShowInFolder = async () => {
    if (!contextMenu) return;
    try {
      console.log("[DocTree] showInFolder:", contextMenu.node.path);
      await wikiService.showInFolder(contextMenu.node.path);
    } catch (e) {
      console.error("[DocTree] showInFolder error:", e);
      alert(`无法打开文件位置: ${e}`);
    }
    setContextMenu(null);
  };

  const handleDelete = async () => {
    if (!contextMenu) return;
    const node = contextMenu.node;
    const label = node.type === "folder" ? `文件夹 "${node.name}"` : `文件 "${node.name}"`;
    if (!confirm(`确定要删除${label}吗？\n\n此操作不可恢复。`)) return;
    try {
      await wikiService.deleteWikiItem(node.path);
      setContextMenu(null);
      loadTree();
    } catch (e) {
      console.error("删除失败:", e);
      alert(`删除失败: ${e}`);
    }
  };

  const handleNewFile = async () => {
    const raw = prompt("文件名:");
    if (raw) {
      try {
        const name = /\.\w+$/.test(raw) ? raw : `${raw}.md`;
        await wikiService.createWikiItem("", name, "file");
        loadTree();
      } catch (e) {
        console.error("创建文件失败:", e);
        alert(`创建失败: ${e}`);
      }
    }
  };

  const handleNewCanvas = async () => {
    const raw = prompt("画板名:");
    if (raw) {
      try {
        const name = /\.\w+$/.test(raw) ? raw : `${raw}.canvas`;
        await wikiService.createWikiItem("", name, "file");
        loadTree();
      } catch (e) {
        console.error("创建画板失败:", e);
        alert(`创建失败: ${e}`);
      }
    }
  };

  const handleCreateRootFolder = async () => {
    const name = prompt("文件夹名:");
    if (name) {
      try {
        await wikiService.createWikiItem("", name, "folder");
        loadTree();
      } catch (e) {
        alert(`创建失败: ${e}`);
      }
    }
  };

  const trimmed = search.trim();
  const filteredTree = trimmed ? filterTree(tree, trimmed) : tree;

  return (
    <div className={styles.tree} ref={treeRef}>
      <div className={styles.searchBox}>
        <input
          id="doctree-search"
          name="doctree-search"
          className={styles.searchInput}
          placeholder="搜索文档..."
          aria-label="搜索文档"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>
      <div className={styles.toolbar}>
        {onUploadClick && (
          <button type="button" className={styles.toolBtn} onClick={onUploadClick} title="上传文件">
            <Upload size={14} />
          </button>
        )}
        <button type="button" className={styles.toolBtn} onClick={handleCreateRootFolder} title="新建文件夹">
          <FolderPlus size={14} />
        </button>
        <button type="button" className={styles.toolBtn} onClick={handleNewFile} title="新建文档">
          <FilePlus size={14} />
        </button>
        <button type="button" className={styles.toolBtn} onClick={handleNewCanvas} title="新建画板">
          <Palette size={14} />
        </button>
      </div>
      <div className={styles.treeContent}>
        {filteredTree.map((node) => (
          <TreeNode
            key={node.path}
            node={node}
            depth={0}
            selectedPath={selectedPath}
            onSelect={handleSelect}
            onRefresh={loadTree}
            onContextMenu={handleContextMenu}
            forceExpand={!!trimmed}
          />
        ))}
        {filteredTree.length === 0 && (
          <div className={styles.empty}>{trimmed ? "无匹配结果" : "暂无文档"}</div>
        )}
      </div>

      {contextMenu && (
        <div
          className={styles.contextMenu}
          style={{ left: contextMenu.x, top: contextMenu.y }}
        >
          <button className={styles.contextMenuItem} onClick={handleRename}>
            <Pencil size={12} /> 重命名
          </button>
          <button className={styles.contextMenuItem} onClick={handleMoveClick}>
            <MoveRight size={12} /> 移动到...
          </button>
          <div className={styles.contextMenuDivider} />
          <button className={styles.contextMenuItem} onClick={handleDelete}>
            <Trash2 size={12} /> 删除
          </button>
          <div className={styles.contextMenuDivider} />
          <button className={styles.contextMenuItem} onClick={handleShowInFolder}>
            <ExternalLink size={12} /> 在资源管理器中显示
          </button>
        </div>
      )}

      {/* Move modal */}
      {moveModal && (
        <div className={styles.modalOverlay} onClick={() => setMoveModal(null)}>
          <div className={styles.moveModal} onClick={(e) => e.stopPropagation()}>
            <div className={styles.moveModalTitle}>
              移动 "{moveModal.node.name}" 到...
            </div>
            <div className={styles.moveModalList}>
              <button
                className={`${styles.moveModalItem} ${moveModal.node.path.split("/").slice(0, -1).join("/") === "" ? styles.moveModalItemActive : ""}`}
                onClick={() => handleMoveConfirm("")}
              >
                根目录
              </button>
              {moveModal.folders.filter(f => f !== "" && f !== moveModal.node.path).map((f) => (
                <button
                  key={f}
                  className={`${styles.moveModalItem} ${moveModal.node.path.split("/").slice(0, -1).join("/") === f ? styles.moveModalItemActive : ""}`}
                  onClick={() => handleMoveConfirm(f)}
                >
                  {f}
                </button>
              ))}
            </div>
            <button className={styles.moveModalCancel} onClick={() => setMoveModal(null)}>
              取消
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
