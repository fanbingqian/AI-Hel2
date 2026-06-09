use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::fs;
use chrono::Datelike;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileNode {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub node_type: String, // "file" | "folder"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_kind: Option<String>, // "md" | "canvas" | "image" | "static" | "convertible"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<FileNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineGroup {
    pub date: String,
    pub label: String,
    pub files: Vec<WikiFileMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiFileMeta {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified: u64,
    pub file_type: String,
    pub title: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FrontMatter {
    pub title: String,
    pub tags: Vec<String>,
    pub created: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WikiUploadResult {
    pub copied: u32,
    pub skipped: u32,
    pub converted: u32,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
}

// Allowed file extensions for wiki
const ALLOWED_EXTENSIONS: &[&str] = &["md", "canvas", "json", "png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "ico", "tiff", "pdf"];
const CONVERTIBLE_EXTENSIONS: &[&str] = &["docx", "xlsx", "pptx"];

pub struct WikiService {
    wiki_dir: PathBuf,
}

impl WikiService {
    pub fn new(wiki_dir: &Path) -> Self {
        Self {
            wiki_dir: wiki_dir.to_path_buf(),
        }
    }

    /// Seed the wiki directory with a default welcome document and folder structure.
    pub fn seed_wiki(&self) -> Result<(), String> {
        fs::create_dir_all(&self.wiki_dir).map_err(|e| format!("创建wiki目录失败: {e}"))?;

        // Create default folders
        for folder in &["笔记", "项目", "日记", "画板"] {
            let dir = self.wiki_dir.join(folder);
            if !dir.exists() {
                fs::create_dir(&dir).map_err(|e| format!("创建目录失败: {e}"))?;
            }
        }

        // Create welcome document
        let welcome_path = self.wiki_dir.join("欢迎使用 AI-Hel2.md");
        if !welcome_path.exists() {
            let content = r#"---
title: 欢迎使用 AI-Hel2
tags: welcome, 入门
created: "2026-05-27"
---

# 欢迎使用 AI-Hel2

这是你的个人知识库。你可以在这里：

- **📝 撰写笔记** — 创建 Markdown 文档，自动提取实体和关系
- **🔗 构建知识图谱** — 实体自动关联，形成你的第二大脑
- **🎨 绘制画板** — 使用 Excalidraw 进行可视化思考
- **💬 与 AI 对话** — 知识库内容会自动作为对话上下文

## 快速开始

1. 在左侧 **文档列表** 中右键新建文件或文件夹
2. 编辑 Markdown 文档，AI 会自动提取关键实体
3. 切换到 **实体浏览** 查看知识图谱
4. 点击 **同步到球体** 将实体推送到 3D 知识球

---
*AI-Hel2 — 你的第二大脑*
"#;
            fs::write(&welcome_path, content).map_err(|e| format!("写入欢迎文档失败: {e}"))?;
        }

        // Create a sample note
        let sample_path = self.wiki_dir.join("笔记").join("示例笔记.md");
        if !sample_path.exists() {
            let content = r#"---
title: 示例笔记
tags: 示例, 笔记
created: "2026-05-27"
---

# 示例笔记

这是一条示例笔记。AI-Hel2 会从你的文档中自动提取实体并构建知识图谱。

## 核心概念

- **实体 (Entity)**: 文档中的关键概念、人物、事件等
- **关系 (Relation)**: 实体之间的连接
- **命名空间 (Namespace)**: 用于组织不同领域的知识

试着编辑这个文件，观察知识图谱的变化！
"#;
            fs::write(&sample_path, content).map_err(|e| format!("写入示例笔记失败: {e}"))?;
        }

        Ok(())
    }

    /// Resolve a relative wiki path to an absolute filesystem path
    pub fn resolve_path(&self, relative_path: &str) -> Result<PathBuf, String> {
        Self::validate_wiki_path(relative_path)?;
        Ok(self.wiki_dir.join(relative_path))
    }

    /// Validate a wiki path against directory traversal and unsafe characters
    pub fn validate_wiki_path(relative_path: &str) -> Result<(), String> {
        if relative_path.is_empty() {
            return Err("路径不能为空".into());
        }
        let path = Path::new(relative_path);
        // Block absolute paths
        if path.is_absolute() {
            return Err("不允许使用绝对路径".into());
        }
        // Block .. traversal
        for component in path.components() {
            if component == std::path::Component::ParentDir {
                return Err("不允许使用 .. 路径遍历".into());
            }
        }
        // Block null bytes and control characters
        if relative_path.contains('\0')
            || relative_path.chars().any(|c| c.is_control() && c != '\n')
        {
            return Err("路径包含非法字符".into());
        }
        // Block unsafe characters
        if relative_path.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*')) {
            return Err("路径包含不安全字符".into());
        }
        // Block hidden files
        if path.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with('.') && n != ".gitkeep")
            .unwrap_or(false)
        {
            return Err("不允许使用隐藏文件".into());
        }
        Ok(())
    }

    fn classify_file_kind(ext: &str) -> Option<&'static str> {
        match ext.to_lowercase().as_str() {
            "md" => Some("md"),
            "canvas" => Some("canvas"),
            "json" => Some("canvas"),
            "docx" | "xlsx" | "pptx" => Some("convertible"),
            "pdf" => Some("pdf"),
            "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "ico" | "tiff" => Some("image"),
            _ => Some("static"),
        }
    }

    pub fn get_file_tree(&self, _namespace: Option<&str>) -> Result<Vec<FileNode>, String> {
        if !self.wiki_dir.exists() {
            return Ok(Vec::new());
        }
        self.build_tree(&self.wiki_dir, &self.wiki_dir)
    }

    fn build_tree(&self, dir: &Path, base: &Path) -> Result<Vec<FileNode>, String> {
        let mut nodes: Vec<FileNode> = Vec::new();
        let mut entries: Vec<_> = fs::read_dir(dir)
            .map_err(|e| format!("读取目录失败: {e}"))?
            .filter_map(|e| e.ok())
            .collect();
        // Sort: folders first, then by name
        entries.sort_by(|a, b| {
            let a_is_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let b_is_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
            b_is_dir.cmp(&a_is_dir).then_with(|| {
                a.file_name()
                    .to_string_lossy()
                    .to_lowercase()
                    .cmp(&b.file_name().to_string_lossy().to_lowercase())
            })
        });

        for entry in entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            // Skip trash dir only
            if name == "_trash" {
                continue;
            }

            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let children = self.build_tree(&path, base).ok();
                nodes.push(FileNode {
                    name,
                    path: rel,
                    node_type: "folder".into(),
                    file_kind: None,
                    children,
                    size: None,
                    modified_at: None,
                    title: None,
                    tags: None,
                });
            } else {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if ALLOWED_EXTENSIONS.contains(&ext.to_lowercase().as_str())
                    || CONVERTIBLE_EXTENSIONS.contains(&ext.to_lowercase().as_str())
                {
                    let meta = entry.metadata().ok();
                    let size = meta.as_ref().map(|m| m.len());
                    let modified_at = meta
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs());
                    let file_kind = Self::classify_file_kind(ext);
                    // Parse frontmatter for .md files to get title/tags
                    let (title, tags) = if ext.eq_ignore_ascii_case("md") {
                        match fs::read_to_string(&path) {
                            Ok(content) => {
                                let fm = Self::parse_frontmatter(&content);
                                let title = if fm.title.is_empty() { None } else { Some(fm.title) };
                                let tags = if fm.tags.is_empty() { None } else { Some(fm.tags) };
                                (title, tags)
                            }
                            Err(_) => (None, None),
                        }
                    } else {
                        (None, None)
                    };
                    nodes.push(FileNode {
                        name,
                        path: rel,
                        node_type: "file".into(),
                        file_kind: file_kind.map(|s| s.to_string()),
                        children: None,
                        size,
                        modified_at,
                        title,
                        tags,
                    });
                }
            }
        }
        Ok(nodes)
    }

    pub fn get_timeline(&self, _namespace: Option<&str>) -> Result<Vec<TimelineGroup>, String> {
        let all = self.list_all_files(None)?;
        let now = chrono::Local::now();
        let today = now.date_naive();
        let yesterday = today - chrono::TimeDelta::days(1);
        let week_start = today - chrono::TimeDelta::days(today.weekday().num_days_from_monday() as i64);
        let month_start = chrono::NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);

        let mut today_files = Vec::new();
        let mut yesterday_files = Vec::new();
        let mut this_week_files = Vec::new();
        let mut this_month_files = Vec::new();
        let mut older_files = Vec::new();

        for f in all {
            let modified = chrono::DateTime::from_timestamp(f.modified as i64, 0)
                .map(|dt| dt.with_timezone(&chrono::Local).date_naive())
                .unwrap_or(today);
            if modified == today {
                today_files.push(f);
            } else if modified == yesterday {
                yesterday_files.push(f);
            } else if modified >= week_start {
                this_week_files.push(f);
            } else if modified >= month_start {
                this_month_files.push(f);
            } else {
                older_files.push(f);
            }
        }

        let mut groups: Vec<TimelineGroup> = Vec::new();
        if !today_files.is_empty() {
            groups.push(TimelineGroup { date: today.to_string(), label: "今天".into(), files: today_files });
        }
        if !yesterday_files.is_empty() {
            groups.push(TimelineGroup { date: yesterday.to_string(), label: "昨天".into(), files: yesterday_files });
        }
        if !this_week_files.is_empty() {
            groups.push(TimelineGroup { date: week_start.to_string(), label: "本周".into(), files: this_week_files });
        }
        if !this_month_files.is_empty() {
            groups.push(TimelineGroup { date: month_start.to_string(), label: "本月".into(), files: this_month_files });
        }
        if !older_files.is_empty() {
            groups.push(TimelineGroup { date: "older".into(), label: "更早".into(), files: older_files });
        }
        Ok(groups)
    }

    pub fn list_files(&self, namespace: Option<&str>) -> Result<Vec<WikiFile>, String> {
        let search_dir = match namespace {
            Some(ns) => self.wiki_dir.join(ns),
            None => self.wiki_dir.clone(),
        };

        if !search_dir.exists() {
            return Ok(Vec::new());
        }

        let mut files = Vec::new();
        self.walk_dir(&search_dir, &mut files, &search_dir)?;
        files.sort_by(|a, b| b.modified.cmp(&a.modified));
        Ok(files)
    }

    fn walk_dir(
        &self,
        dir: &Path,
        files: &mut Vec<WikiFile>,
        base: &Path,
    ) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|e| format!("读取目录失败: {e}"))? {
            let entry = entry.map_err(|e| format!("读取条目失败: {e}"))?;
            let path = entry.path();

            if path.is_dir() {
                if path.file_name().map_or(false, |n| n == "_trash") {
                    continue;
                }
                self.walk_dir(&path, files, base)?;
            } else if path.extension().map_or(false, |ext| ext == "md") {
                let meta = entry.metadata().map_err(|e| format!("读取元数据失败: {e}"))?;
                let rel = path
                    .strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                files.push(WikiFile {
                    name: path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    path: rel,
                    size: meta.len(),
                    modified: meta
                        .modified()
                        .map(|t| {
                            t.duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs()
                        })
                        .unwrap_or(0),
                });
            }
        }
        Ok(())
    }

    pub fn upload_file(
        &self,
        source_path: &str,
        target_dir: Option<&str>,
        target_name: Option<&str>,
    ) -> Result<String, String> {
        let source = Path::new(source_path);
        if !source.exists() {
            return Err(format!("文件不存在: {source_path}"));
        }
        if !source.is_file() {
            return Err(format!("路径不是文件: {source_path}"));
        }

        // Validate extension
        let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) && !CONVERTIBLE_EXTENSIONS.contains(&ext.as_str()) {
            return Err(format!("不支持的文件类型: .{ext}"));
        }

        let dest_dir = match target_dir {
            Some(dir) => self.wiki_dir.join(dir),
            None => self.wiki_dir.clone(),
        };
        fs::create_dir_all(&dest_dir).map_err(|e| format!("创建目录失败: {e}"))?;

        let name = target_name.unwrap_or_else(|| source.file_name().unwrap_or_default().to_str().unwrap_or("untitled"));
        let dest = dest_dir.join(name);

        // Skip if same file (by size + modified time)
        if dest.exists() {
            let src_meta = fs::metadata(source).map_err(|e| format!("读取源文件失败: {e}"))?;
            let dst_meta = fs::metadata(&dest).map_err(|e| format!("读取目标文件失败: {e}"))?;
            if src_meta.len() == dst_meta.len() {
                return Err(format!("文件已存在且大小相同: {}", name));
            }
        }

        fs::copy(source, &dest).map_err(|e| format!("复制文件失败: {e}"))?;

        // Build relative wiki path
        let rel = dest.strip_prefix(&self.wiki_dir).unwrap_or(&dest);
        Ok(rel.to_string_lossy().replace('\\', "/"))
    }

    pub fn upload_files(
        &self,
        paths: &[String],
        target_dir: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let mut results = Vec::new();
        let mut first_err: Option<String> = None;

        for path in paths {
            match self.upload_file(path, target_dir, None) {
                Ok(rel) => results.push(rel),
                Err(e) => {
                    if first_err.is_none() { first_err = Some(e); }
                }
            }
        }

        if results.is_empty() {
            Err(first_err.unwrap_or_else(|| "没有文件被上传".into()))
        } else {
            Ok(results)
        }
    }

    pub fn upload_folder(
        &self,
        source_path: &str,
        namespace: Option<&str>,
    ) -> Result<WikiUploadResult, String> {
        let source = Path::new(source_path);
        if !source.exists() {
            return Err(format!("路径不存在: {source_path}"));
        }
        if !source.is_dir() {
            return Err(format!("路径不是目录: {source_path}"));
        }

        let dest_dir = match namespace {
            Some(ns) => self.wiki_dir.join(ns),
            None => self.wiki_dir.clone(),
        };
        fs::create_dir_all(&dest_dir).map_err(|e| format!("创建目录失败: {e}"))?;

        let mut result = WikiUploadResult {
            copied: 0,
            skipped: 0,
            converted: 0,
            errors: Vec::new(),
        };

        Self::upload_dir_recursive(source, &dest_dir, source, &mut result);

        Ok(result)
    }

    fn upload_dir_recursive(
        dir: &Path,
        base_dest: &Path,
        base_src: &Path,
        result: &mut WikiUploadResult,
    ) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                result.errors.push(format!("读取目录 {} 失败: {e}", dir.display()));
                return;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    result.errors.push(e.to_string());
                    continue;
                }
            };
            let path = entry.path();

            if path.is_dir() {
                Self::upload_dir_recursive(&path, base_dest, base_src, result);
                continue;
            }

            if !path.is_file() {
                continue;
            }

            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

            match ext {
                "md" => {
                    let relative = path.strip_prefix(base_src).unwrap_or(&path);
                    let dest = base_dest.join(relative);
                    if let Some(parent) = dest.parent() {
                        if let Err(e) = fs::create_dir_all(parent) {
                            result.errors.push(format!("创建目录 {} 失败: {e}", parent.display()));
                            continue;
                        }
                    }
                    if dest.exists() {
                        result.skipped += 1;
                        continue;
                    }
                    match fs::copy(&path, &dest) {
                        Ok(_) => result.copied += 1,
                        Err(e) => result.errors.push(format!("{}: {e}", path.display())),
                    }
                }
                _ => {
                    result.skipped += 1;
                }
            }
        }
    }

    pub fn read_file(&self, file_path: &str) -> Result<String, String> {
        Self::validate_wiki_path(file_path)?;
        let target = self.wiki_dir.join(file_path);
        if !target.exists() {
            return Err(format!("文件不存在: {file_path}"));
        }
        fs::read_to_string(&target).map_err(|e| format!("读取文件失败: {e}"))
    }

    pub fn read_file_base64(&self, file_path: &str) -> Result<String, String> {
        Self::validate_wiki_path(file_path)?;
        let target = self.wiki_dir.join(file_path);
        if !target.exists() {
            return Err(format!("文件不存在: {file_path}"));
        }
        use base64::Engine;
        let bytes = fs::read(&target).map_err(|e| format!("读取文件失败: {e}"))?;
        Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
    }

    pub fn write_file(&self, file_path: &str, content: &str) -> Result<(), String> {
        Self::validate_wiki_path(file_path)?;
        let target = self.wiki_dir.join(file_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        }
        fs::write(&target, content).map_err(|e| format!("写入文件失败: {e}"))
    }

    pub fn create_file(&self, file_path: &str) -> Result<(), String> {
        Self::validate_wiki_path(file_path)?;
        let target = self.wiki_dir.join(file_path);
        if target.exists() {
            return Err(format!("文件已存在: {file_path}"));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        }
        let content = if file_path.ends_with(".md") {
            format!("# {}\n\n开始写作...\n",
                std::path::Path::new(file_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("新文档"))
        } else {
            String::new()
        };
        fs::write(&target, &content).map_err(|e| format!("创建文件失败: {e}"))
    }

    pub fn create_dir(&self, dir_path: &str) -> Result<(), String> {
        Self::validate_wiki_path(dir_path)?;
        let target = self.wiki_dir.join(dir_path);
        if target.exists() {
            return Err(format!("目录已存在: {dir_path}"));
        }
        fs::create_dir_all(&target).map_err(|e| format!("创建目录失败: {e}"))
    }

    pub fn rename_file(&self, old_path: &str, new_path: &str) -> Result<WikiFileMeta, String> {
        self.move_file(old_path, new_path)
    }

    pub fn list_dirs(&self, namespace: Option<&str>) -> Result<Vec<String>, String> {
        let search_dir = match namespace {
            Some(ns) => self.wiki_dir.join(ns),
            None => self.wiki_dir.clone(),
        };
        if !search_dir.exists() {
            return Ok(Vec::new());
        }
        let mut dirs = Vec::new();
        for entry in fs::read_dir(&search_dir).map_err(|e| format!("读取目录失败: {e}"))? {
            let entry = entry.map_err(|e| format!("读取条目失败: {e}"))?;
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(name) = entry.path().file_name() {
                    let n = name.to_string_lossy();
                    if n != "_trash" {
                        dirs.push(n.into_owned());
                    }
                }
            }
        }
        dirs.sort();
        Ok(dirs)
    }

    pub fn delete_file(&self, file_path: &str, soft: bool) -> Result<(), String> {
        Self::validate_wiki_path(file_path)?;
        let target = self.wiki_dir.join(file_path);
        if !target.exists() {
            return Err(format!("文件不存在: {file_path}"));
        }

        if soft {
            let trash_dir = self.wiki_dir.join("_trash");
            fs::create_dir_all(&trash_dir).map_err(|e| format!("创建回收站失败: {e}"))?;
            let name = target.file_name().unwrap_or_default();
            let dest = trash_dir.join(name);
            fs::rename(&target, &dest).map_err(|e| format!("移动到回收站失败: {e}"))?;
        } else {
            fs::remove_file(&target).map_err(|e| format!("删除失败: {e}"))?;
        }
        Ok(())
    }

    pub fn get_image_index(&self, _namespace: Option<&str>) -> Result<Vec<ImageEntry>, String> {
        let index_path = self.wiki_dir.join("_图片索引.md");
        if !index_path.exists() {
            return Ok(Vec::new());
        }
        // Parse image index from markdown
        Ok(Vec::new())
    }

    pub fn parse_frontmatter(content: &str) -> FrontMatter {
        let mut fm = FrontMatter::default();
        let trimmed = content.trim_start();
        if !trimmed.starts_with("---") {
            return fm;
        }
        let after_first = &trimmed[3..];
        let end = after_first.find("\n---");
        if let Some(end_idx) = end {
            let fm_text = &after_first[..end_idx];
            for line in fm_text.lines() {
                let line = line.trim();
                if let Some((key, value)) = line.split_once(':') {
                    let key = key.trim().to_lowercase();
                    let value = value.trim().trim_matches('"').trim_matches('\'');
                    match key.as_str() {
                        "title" => fm.title = value.to_string(),
                        "tags" => {
                            fm.tags = value
                                .split(',')
                                .map(|t| t.trim().trim_matches('"').trim_matches('\'').to_string())
                                .filter(|t| !t.is_empty())
                                .collect();
                        }
                        "created" | "date" => fm.created = value.to_string(),
                        _ => {}
                    }
                }
            }
        }
        fm
    }

    fn read_file_meta(&self, rel_path: &str, file_type: &str) -> Option<WikiFileMeta> {
        let target = self.wiki_dir.join(rel_path);
        if !target.exists() {
            return None;
        }
        let meta = target.metadata().ok()?;
        let name = target.file_name()?.to_string_lossy().into_owned();
        let modified = meta
            .modified()
            .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
            .unwrap_or(0);

        let (title, tags) = if file_type == "md" {
            let content = fs::read_to_string(&target).unwrap_or_default();
            let fm = Self::parse_frontmatter(&content);
            let title = if fm.title.is_empty() {
                name.trim_end_matches(".md").to_string()
            } else {
                fm.title
            };
            (title, fm.tags)
        } else {
            (name.trim_end_matches(".json").to_string(), Vec::new())
        };

        Some(WikiFileMeta {
            path: rel_path.to_string(),
            name,
            size: meta.len(),
            modified,
            file_type: file_type.to_string(),
            title,
            tags,
        })
    }

    pub fn list_all_files(&self, namespace: Option<&str>) -> Result<Vec<WikiFileMeta>, String> {
        let search_dir = match namespace {
            Some(ns) => self.wiki_dir.join(ns),
            None => self.wiki_dir.clone(),
        };

        if !search_dir.exists() {
            return Ok(Vec::new());
        }

        let mut files = Vec::new();
        self.walk_all_files(&search_dir, &mut files, &search_dir)?;
        files.sort_by(|a, b| b.modified.cmp(&a.modified));
        Ok(files)
    }

    fn walk_all_files(
        &self,
        dir: &Path,
        files: &mut Vec<WikiFileMeta>,
        base: &Path,
    ) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|e| format!("读取目录失败: {e}"))? {
            let entry = entry.map_err(|e| format!("读取条目失败: {e}"))?;
            let path = entry.path();

            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name == "_trash" {
                    continue;
                }
                self.walk_all_files(&path, files, base)?;
            } else if path.extension().map_or(false, |ext| ext == "md") {
                let rel = path
                    .strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();

                if let Some(meta) = self.read_file_meta(&rel, "md") {
                    files.push(meta);
                }
            }
        }
        Ok(())
    }

    pub fn search_files(
        &self,
        query: &str,
        namespace: Option<&str>,
    ) -> Result<Vec<WikiFileMeta>, String> {
        let all = self.list_all_files(namespace)?;
        let q = query.to_lowercase();
        let results: Vec<WikiFileMeta> = all
            .into_iter()
            .filter(|f| {
                f.name.to_lowercase().contains(&q)
                    || f.title.to_lowercase().contains(&q)
                    || f.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .collect();
        Ok(results)
    }

    pub fn get_all_tags(&self, namespace: Option<&str>) -> Result<Vec<String>, String> {
        let all = self.list_all_files(namespace)?;
        let mut tag_set: HashSet<String> = HashSet::new();
        for f in &all {
            for t in &f.tags {
                if !t.is_empty() {
                    tag_set.insert(t.clone());
                }
            }
        }
        let mut tags: Vec<String> = tag_set.into_iter().collect();
        tags.sort();
        Ok(tags)
    }

    pub fn get_files_by_tag(
        &self,
        tag: &str,
        namespace: Option<&str>,
    ) -> Result<Vec<WikiFileMeta>, String> {
        let all = self.list_all_files(namespace)?;
        let results: Vec<WikiFileMeta> = all
            .into_iter()
            .filter(|f| f.tags.iter().any(|t| t == tag))
            .collect();
        Ok(results)
    }

    pub fn move_file(&self, old_path: &str, new_path: &str) -> Result<WikiFileMeta, String> {
        Self::validate_wiki_path(old_path)?;
        Self::validate_wiki_path(new_path)?;
        let src = self.wiki_dir.join(old_path);
        if !src.exists() {
            return Err(format!("文件不存在: {old_path}"));
        }
        let dest = self.wiki_dir.join(new_path);
        if dest.exists() {
            return Err(format!("目标已存在: {new_path}"));
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        }
        fs::rename(&src, &dest).map_err(|e| format!("移动失败: {e}"))?;

        let file_type = if new_path.ends_with(".md") { "md" } else { "file" };
        self.read_file_meta(new_path, file_type)
            .ok_or_else(|| "移动成功但无法读取元数据".into())
    }

    pub fn update_frontmatter(
        &self,
        file_path: &str,
        title: Option<&str>,
        tags: Option<Vec<String>>,
    ) -> Result<(), String> {
        let target = self.wiki_dir.join(file_path);
        if !target.exists() {
            return Err(format!("文件不存在: {file_path}"));
        }
        let content = fs::read_to_string(&target).map_err(|e| format!("读取文件失败: {e}"))?;

        let (old_fm, body_start) = if content.trim_start().starts_with("---") {
            let trimmed = content.trim_start();
            let after_first = &trimmed[3..];
            if let Some(end_idx) = after_first.find("\n---") {
                let _fm_text = &after_first[..end_idx];
                let start_in_original = content.find("---").unwrap_or(0);
                let end_in_original = content[start_in_original..]
                    .find("\n---")
                    .map(|i| start_in_original + i + 4)
                    .unwrap_or(0);
                (Self::parse_frontmatter(&content), end_in_original)
            } else {
                (FrontMatter::default(), 0)
            }
        } else {
            (FrontMatter::default(), 0)
        };

        let mut fm = old_fm;
        if let Some(t) = title {
            fm.title = t.to_string();
        }
        if let Some(t) = tags {
            fm.tags = t;
        }

        let body = if body_start > 0 {
            content[body_start..].to_string()
        } else {
            content
        };

        let mut new_content = String::from("---\n");
        if !fm.title.is_empty() {
            new_content.push_str(&format!("title: \"{}\"\n", fm.title));
        }
        if !fm.tags.is_empty() {
            let tag_str = fm
                .tags
                .iter()
                .map(|t| format!("\"{}\"", t))
                .collect::<Vec<_>>()
                .join(", ");
            new_content.push_str(&format!("tags: [{}]\n", tag_str));
        }
        if !fm.created.is_empty() {
            new_content.push_str(&format!("created: \"{}\"\n", fm.created));
        }
        new_content.push_str("---\n");
        new_content.push_str(&body.trim_start());

        fs::write(&target, &new_content).map_err(|e| format!("写入文件失败: {e}"))
    }
}
