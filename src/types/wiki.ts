export interface FileNode {
  name: string;
  path: string;
  type: "file" | "folder";
  children?: FileNode[];
  file_kind?: string;
  title?: string;
  namespace?: string;
  tags?: string[];
  size?: number;
  modified_at?: number;
}
