export { getWikiFileTree, readWikiFile, readWikiFileBase64, writeWikiFile, createWikiItem, deleteWikiItem, renameWikiItem, moveWikiItem, showInFolder, listWikiDirs, listAllKnowledgeFiles } from "./api";

export function resolveDisplayPath(path: string): string {
  const parts = path.replace(/\\/g, "/").split("/");
  const wikiIdx = parts.findIndex((p) => p === "wiki");
  if (wikiIdx >= 0) return parts.slice(wikiIdx + 1).join("/");
  return parts.slice(-2).join("/");
}
