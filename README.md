# CineForge 创剧

从剧本到电影的流线化产出工具。CineForge 是 Tauri 2 桌面应用，集成 AI 大模型，覆盖剧本创作、视觉设计、视频生成全流程。

## 快速开始

### 环境要求

- **Rust** 1.80+（[rustup](https://rustup.rs) 安装）
- **Node.js** 20+（[nvm](https://github.com/nvm-sh/nvm) 或 [fnm](https://github.com/Schniz/fnm) 安装）
- **macOS** 14+ / **Windows** 10+ / **Linux**（Wayland 或 X11）

### 安装与运行

```bash
# 1. 克隆仓库
git clone https://github.com/innovationpuls-creator/rerust.git
cd rerust

# 2. 安装前端依赖
npm install

# 3. 启动开发模式（前端 + Rust 后端联动）
npm run tauri dev
```

首次启动会自动创建 SQLite 数据库（位于系统应用数据目录）。

### Rust 后端命令

```bash
cd src-tauri

cargo check           # 类型检查（快速）
cargo clippy          # lint 检查
cargo test            # 运行单元测试
cargo build           # 完整构建
cargo build --release # 生产构建
```

## 架构总览

```
src-tauri/src/
├── main.rs              # Tauri 入口
├── lib.rs               # 命令注册（60+ Tauri commands）
├── db/
│   ├── mod.rs           # 数据库初始化（SQLite + WAL 模式）
│   ├── schema.rs        # 表结构定义
│   └── crud.rs          # 数据访问层
├── llm/
│   ├── mod.rs
│   ├── config.rs        # LLM 配置（endpoint、key、model）
│   ├── prompts.rs       # Prompt 模板加载
│   ├── prompts/         # 各步骤的提示词模板（.txt）
│   └── server_proxy.rs  # LLM API 代理（OpenAI 兼容协议）
├── services/
│   ├── screenplay.rs         # 剧本创作工作流（8 步法）
│   ├── screenplay_store.rs   # 剧本项目持久化
│   ├── script_generation.rs  # 剧本生成服务
│   ├── script_review.rs      # 剧本评审
│   ├── asset_extraction.rs   # 角色/场景/道具提取
│   ├── prompt_generation.rs  # 分镜提示词生成
│   ├── duration.rs           # 时长规格
│   ├── seedance_service.rs   # Seedance 视频生成服务
│   └── seedance_store.rs     # Seedance 数据持久化
└── utils/
    ├── mod.rs
    ├── step_parser.rs    # 步骤输出解析
    └── v5_parser.rs      # V5 格式 markdown 解析
```

## 功能模块

### 1. 剧本创作（Screenplay）

8 步结构化创作流程，每步支持 AI 生成、自检、版本管理、回滚：

| 步骤 | 名称 | 说明 |
|------|------|------|
| 1 | 核心概念 | 故事内核与高概念提炼 |
| 2 | 三幕结构 | 经典三幕剧结构搭建 |
| 3 | 角色设定 | 主要角色性格与弧光设计 |
| 4 | 场景大纲 | 分场景叙事框架 |
| 5 | 剧本初稿 | 场景级剧本正文 |
| 6 | 分镜规划 | 镜头语言与视觉节奏 |
| 7 | 视觉风格 | 画面风格与美学定位 |
| 8 | 最终定稿 | 整合输出完整剧本包 |

### 2. 剧本快速生成（Script Generation）

基于情节摘要一键生成完整剧本，支持体裁、受众、语调、结局等参数配置。

### 3. 图像生成（Image Generation）

根据剧本内容或自定义描述生成视觉素材，支持风格化参数配置。

### 4. 视频生成 / Seedance

Seedance 视频生成管线：
- **Phase AD** — 分析与设计阶段
- **Unit EFG** — 逐单元执行与生成

### 5. 资产提取（Asset Extraction）

从剧本中自动提取角色、场景、道具等结构化数据。

### 6. 提示词生成（Prompt Generation）

为图像/视频生成模型自动构建高质量提示词，支持分镜级精确控制。

### 7. 剧本评审（Script Review）

AI 驱动的剧本质量评审，提供结构化反馈。

### 8. 项目管理

项目管理支持创建、重命名、删除项目，以及剧本任务、图像任务、视频任务的历史回顾。

### 9. 用户认证

本地用户注册/登录，SHA-256 密码哈希，基于 token 的会话管理。

## LLM 配置

CineForge 兼容 OpenAI API 协议的 LLM 服务。在应用内配置：

- **Endpoint** — API 地址
- **API Key** — 访问密钥
- **Model** — 模型名称
- **Mode** — openai / custom

支持文本生成和图像生成接口的独立配置与连接测试。

## 技术栈

| 层面 | 技术 |
|------|------|
| 桌面框架 | Tauri 2 |
| 后端语言 | Rust 2021 edition |
| 数据库 | SQLite（rusqlite + bundled） |
| HTTP 客户端 | reqwest（rustls-tls）+ 流式响应 |
| 异步运行时 | tokio（full features） |
| 序列化 | serde + serde_json |
| 前端 | Tauri WebView（dist/ 静态资源） |

## 项目结构（关键文件）

```
rerust/
├── src-tauri/
│   ├── Cargo.toml          # Rust 依赖与元数据
│   ├── tauri.conf.json     # Tauri 窗口、打包、安全配置
│   ├── capabilities/       # Tauri 权限声明
│   ├── icons/              # 应用图标
│   └── src/                # Rust 源码
├── dist/                   # 前端构建产物（Tauri 直接加载）
├── package.json            # Node 依赖（Tauri CLI）
└── CLAUDE.md               # Claude Code 协作说明
```

## 版本

当前版本：**2.0.5**

## License

Private — All rights reserved.
