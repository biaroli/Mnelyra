<p align="center">
  <img src="static/favicon.png" width="108" alt="RootRelay 图标">
</p>

<h1 align="center">RootRelay</h1>

<p align="center">
  用一个稳定的 MCP 地址，把本地工作区接给兼容 MCP 的客户端。
</p>

<p align="center">
  <a href="README.md">English</a>&nbsp;&nbsp;<a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="https://github.com/biaroli/RootRelay/releases/latest"><img src="https://img.shields.io/github/v/release/biaroli/RootRelay?label=release" alt="release"></a>
  <img src="https://img.shields.io/badge/Windows-x64-0078D4?logo=windows" alt="Windows x64">
  <img src="https://img.shields.io/badge/macOS-universal-000000?logo=apple" alt="macOS universal">
  <img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="Apache-2.0">
</p>

RootRelay 在你的电脑上运行 MCP 服务，并把它指向当前选择的工作区。连接后的客户端可以读取和修改文件、搜索项目、执行允许的命令和测试、检查 Git、查看图片，并保存跟随项目走的开发历史。

客户端连接和工作区是分开的。多个项目导入一次后，点击左侧项目就会切换 MCP 根目录并立即激活，不需要换公网地址，也不需要重新配置一套服务。

![RootRelay 工作区预览](static/readme/rootrelay-workspace.svg)

预览图中的项目名、路径、域名和凭据全部是虚构内容。

## 关于

RootRelay 是一个面向本地开发项目的桌面 MCP 工作区中继。它保持同一套 MCP 服务身份和客户端连接地址，同时允许你直接从桌面界面切换连接背后的项目根目录。

它解决的是一个很直接的问题：工作区只需要导入一次，MCP 客户端只需要连接一次，之后切项目不用反复重配 endpoint。RootRelay 负责这条连接背后的工作区边界、MCP 工具、认证、隧道生命周期和跟随项目保存的会话历史。

## RootRelay 能做什么

| 部分 | 行为 |
| --- | --- |
| 工作区 | 导入本地项目目录，从侧栏直接切换当前项目根目录 |
| MCP | 提供 Streamable HTTP MCP，以及文件、搜索、Patch、命令、Git、图片和历史工具 |
| 客户端 | 只要客户端支持对应的 MCP 传输方式和认证方式，就可以连接 RootRelay |
| 公网接入 | 支持 Cloudflare Named Tunnel、Cloudflare Quick Tunnel、FRP，也可以只在本机运行 |
| 认证 | 支持 OAuth、Bearer Token 和本机无认证模式 |
| 固定身份 | 安装级 OAuth Client ID 在重启、切工作区和正常维护期间保持不变 |
| 历史会话 | 开发历史保存在项目自己的 `.rootrelay/history-session/` |
| Actions | 需要时可以单独开放 OpenAPI Actions 网关 |

## MCP 客户端

RootRelay 不绑定某一家 AI。ChatGPT 和 Claude 都是常见的远程 MCP 客户端；其他支持 Streamable HTTP MCP 和当前认证方式的客户端，也可以使用同一个 RootRelay endpoint。

不同客户端开放的 MCP 能力和权限不完全相同。RootRelay 负责服务端和工具，最终能调用哪些能力由客户端本身决定。

## 安装

从 [GitHub Releases](https://github.com/biaroli/RootRelay/releases/latest) 下载当前版本。

| 系统 | 安装包 |
| --- | --- |
| Windows 10/11 x64 | `RootRelay_*_x64-setup.exe` |
| macOS Intel + Apple Silicon | `RootRelay_*_universal.dmg` |

macOS 当前没有 Apple Developer notarization。第一次打开时，系统可能要求在 系统设置 → 隐私与安全性 中确认。

## 第一次配置

### 1. 配置 MCP 服务

打开 通用，设置本地端口、权限模式、允许执行的命令和需要使用的隧道方式，然后保存。这套配置对所有工作区生效，新项目不需要重新填写一遍。

只给本机 MCP 客户端使用时，可以关闭公网隧道，地址类似：

```text
http://127.0.0.1:28766/mcp
```

### 2. 配置认证

打开 认证。公网 MCP 推荐使用 OAuth。

RootRelay 首次初始化时生成一个安装级 OAuth Client ID，并在整个安装生命周期中保持固定。界面只读显示 Client ID；授权口令和其他 Secret 仍然可以单独轮换。

### 3. 导入工作区

点击 添加工作区，选择项目根目录。

导入后，点击任意工作区会完整执行：

```text
保存当前选择
→ 停止旧工作区 MCP
→ 切换项目根目录
→ 使用同一套全局配置激活新工作区
```

选完项目以后不需要再点一次启动。

## 公网接入

### Cloudflare Named Tunnel

长期连接建议使用 Named Tunnel。配置 Tunnel Token 和一个固定 HTTPS 域名，例如 `https://mcp.example.com`，客户端使用：

```text
https://mcp.example.com/mcp
```

切换项目时，固定域名和 OAuth Client ID 都不会改变。RootRelay 会等待新的 Tunnel 真正可用，再处理旧连接。

### Cloudflare Quick Tunnel

Quick Tunnel 适合临时测试。它使用 `trycloudflare.com` 临时地址，重启以后地址可能变化，不适合长期保存为客户端 endpoint。

### FRP

已有 FRPS 服务端时，在 FRP 配置 保存服务器地址、端口和 Token。然后回到 通用，把 MCP 隧道切到 FRP，并填写需要使用的子域名。

## 连接客户端

把 RootRelay 工作区页面显示的 `/mcp` 地址填进支持自定义 MCP 的客户端，并选择和 RootRelay 一致的认证方式。

第一次连接可以检查：

```text
server_info
get_default_cwd
git_status
```

`server_info` 用来确认 RootRelay 服务，`get_default_cwd` 用来确认当前项目根目录，`git_status` 用来确认 Git 已经指向预期仓库。

## 工作区切换

端口、认证、隧道和执行策略属于应用级配置。工作区只决定当前 MCP 指向哪个项目根目录。

因此切换项目时，客户端可以继续使用原来的公网地址和 OAuth 身份，RootRelay 只替换 endpoint 后面的工作区。

应用重启后会恢复上次选择的工作区，并重新启动已经配置好的 MCP 服务。

## 历史会话

RootRelay 可以把开发上下文保存在项目内部：

```text
.rootrelay/history-session/
```

| 工具 | 用途 |
| --- | --- |
| `history_session_bootstrap` | 新对话初始化或恢复当前历史会话 |
| `history_session_checkpoint` | 保存本轮决策、改动、验证和下一步 |
| `history_session_search` | 搜索旧会话 |
| `history_session_read` | 读取指定历史档案 |
| `history_session_validate` | 检查历史编号和派生索引 |

RootRelay 不会在后台读取远程聊天窗口。只有 MCP 客户端实际调用历史工具时，内容才会写入项目。

## 安全

RootRelay 可以修改当前工作区文件并执行命令。公网 endpoint 应开启认证，只连接自己控制或明确可信的客户端。

Windows 当前的命令安全主要由 RootRelay 的工作区边界和命令策略提供，不能当成完整的操作系统级文件沙箱。

## 开发

维护者环境和工程说明放在 [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)。

## License

RootRelay 使用 [Apache License 2.0](LICENSE)。

RootRelay 最初基于 [mybolide/coding-tools-mcp](https://github.com/mybolide/coding-tools-mcp) 继续开发；该项目本身 fork 自 Coding Tools MCP 原项目。感谢相关贡献者提供早期 Apache-2.0 代码基础。Copyright 2026 Coding Tools MCP Contributors。

RootRelay 与 OpenAI、Anthropic、Cloudflare 没有隶属或官方合作关系。
