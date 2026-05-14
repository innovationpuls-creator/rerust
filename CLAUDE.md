# CLAUDE.md

## 项目概述

**CineForge（创剧）** — Tauri 2 桌面应用，实现从剧本到电影的流线化产出。

> 完整文档见 [README.md](./README.md)

- **前端**：Tauri WebView（`dist/` 目录下为前端构建产物）
- **后端**：Rust（`src-tauri/`），SQLite 本地数据库，LLM 对话集成
- **构建**：`cargo build`（Rust 侧）；`npm run tauri dev`（开发模式）

## 开发命令

```bash
# Rust 后端
cd src-tauri
cargo check           # 类型检查（快速）
cargo clippy          # lint
cargo test            # 单元测试
cargo build           # 完整构建

# 开发模式（前端 + 后端联动）
npm run tauri dev
```

## 架构要点

```
src-tauri/src/
├── main.rs       # Tauri 入口，注册命令和插件
├── lib.rs        # 库入口
├── db/           # SQLite 数据层
├── llm/          # LLM 调用与流式响应处理
├── services/     # 业务逻辑
└── utils/        # 工具函数
```

- 本地数据库：SQLite（rusqlite），数据存在用户目录
- HTTP 请求：reqwest（rustls-tls），支持流式响应（LLM 对话）
- 异步运行时：tokio
- 序列化：serde + serde_json

## 协作模式切换

本项目涉及产品设计（剧本工作流、AI 辅助功能规划）与工程实现（Rust + Tauri 编码）两类工作，使用不同技能完成：

| 阶段 | 使用的技能 | 说明 |
|------|-----------|------|
| 功能设计 / 工作流探索 / 方案比较 | `brainstorming` | 讨论剧本处理流程、AI 能力边界、UI 交互方案等 |
| Rust 编码 / Tauri 命令开发 / 前端实现 | ECC 内置能力 | 按照 Tauri 2 + Rust 约定直接实现 |

**重要**：两个阶段是独立触发的。brainstorming 完成后不会自动进入 writing-plans 或 TDD 流程——需要你明确切换到编码模式后，才会开始实现。
