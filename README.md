# AI-Hel2 — 本地知识桌面助手

一个基于 **Tauri v2** 的桌面应用，将 **AI 对话** 和 **个人知识图谱** 融合在同一界面中。你的聊天内容自动转化为结构化知识，可搜索、可关联、可演化。

## 核心能力

### 对话
- 对接 Hermes Agent v0.15.2（内嵌 Python 运行时）
- 支持 DeepSeek / OpenAI / Anthropic 等任意 OpenAI 兼容 API
- 流式对话 + 思考过程展示 + 工具调用时间线
- 按住右 Alt 语音输入（PTT）+ 语音播报

### 知识图谱（Nexus 引擎）
- **自动提取**：文档和对话内容通过 LLM 自动提取实体和关系
- **Barnes-Hut 四叉树渲染**：对齐 Obsidian 物理引擎，力导向布局
- **文档折叠视图**：一键收起所有实体，只看文档关系网
- **6 组维护操作**：健康检查 / 去重合并 / 文档归类 / 图谱分析 / 传递推理 / 冲突检测
- **推断实体**：LLM 驱动的跨文档推理，自动发现隐藏关联

### 知识管理
- Wiki 文档树：Markdown 编辑（Cherry）+ 文件预览 + 拖拽上传
- 全文搜索 + 实体详情面板
- 图表例自定义颜色、缩放淡出、连线透明度

### 智能体平台
- 多 Agent 注册与健康监控
- API 配置向导（首次启动引导）
- 知识引擎 LLM 独立配置（支持从 Agent 配置一键复制）

## 技术架构

```
┌─ Tauri v2 Shell (Rust) ────────────────────────────┐
│  窗口管理 / 系统托盘 / 全局快捷键 / 文件监听          │
├─ Frontend (React + TypeScript + Vite) ──────────────┤
│  D3-force 图谱 / Cherry Markdown / Excalidraw 画板   │
├─ Nexus Knowledge Engine (Rust + Python) ────────────┤
│  SQLite 缓存 / LLM 提取 / Barnes-Hut 物理 / 去重     │
├─ Hermes Agent v0.15.2 (Python, embedded) ───────────┤
│  AI 对话 / 工具调用 / 网页搜索 / 知识库插件            │
└──────────────────────────────────────────────────────┘
```

## 安装

从 [Releases](https://github.com/fanbingqian/AI-Hel2/releases) 下载最新 `AI-Hel2_x.x.x_x64-setup.exe`，一键安装。

### 安装后配置
1. 打开应用 → 注册账号 → 进入 API 配置向导
2. 填入大模型 API Key（DeepSeek / OpenAI 等）
3. Agent 自动启动 → 开始对话
4. 知识库会自动初始化，Agent 已知知识库工具

### 系统要求
- Windows 10/11 x64
- 不需要额外安装 Python 或 Git（已内置）

## 开发

```bash
# 安装依赖
npm install

# 开发模式
npm run tauri dev

# 构建安装包
npm run tauri build
# → src-tauri/target/release/bundle/nsis/AI-Hel2_x.x.x_x64-setup.exe
```

### 签名构建
```bash
# 先生成签名密钥（只需一次）
npx tauri signer generate -w src-tauri/updater-key
# 提示密码时直接回车（空密码）

# 构建并签名
$env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content src-tauri/updater-key -Raw).Trim()
npm run tauri build
```

## 项目结构

```
AI-Hel2/
├── src/                     # React 前端
│   ├── components/          # UI 组件
│   │   ├── chat/            # 聊天面板
│   │   ├── sphere/          # 知识图谱（物理引擎 + 渲染）
│   │   ├── knowledge/       # 文档编辑 + 实体浏览
│   │   ├── settings/        # 设置页
│   │   └── auth/            # 登录注册
│   ├── stores/              # Zustand 状态管理
│   ├── services/            # API 调用
│   └── types/               # TypeScript 类型
├── src-tauri/               # Rust 后端
│   ├── src/commands/        # Tauri 命令
│   ├── src/services/        # 核心服务（知识引擎等）
│   ├── src/models/          # 数据模型
│   ├── hermes-agent/        # 内嵌 Agent（构建时打包）
│   └── migrations/          # SQLite 迁移
└── docs/                    # 设计文档
```

## License

MIT
