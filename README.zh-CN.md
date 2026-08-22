<p align="center">
  <img src="static/favicon.png" width="108" alt="Mnelyra 图标">
</p>

<h1 align="center">Mnelyra</h1>

<h3 align="center">让 ChatGPT 连接本地工作区，让 Codex 使用 ChatGPT Web 推理。</h3>

<p align="center">
  <strong>网页端可以直接调用本地文件、命令、Git 和测试工具；Codex 可以把 GPT-5.6 Sol 路由到已登录的 ChatGPT Web 会话。Mnelyra 负责连接、工作区记忆和上下文设置。</strong>
</p>

<p align="center">
  <a href="README.md">English</a>&nbsp;&nbsp;<a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="https://github.com/biaroli/Mnelyra/releases/latest"><img src="https://img.shields.io/github/v/release/biaroli/Mnelyra?label=release" alt="release"></a>
  <img src="https://img.shields.io/badge/Windows-x64-0078D4?logo=windows" alt="Windows x64">
  <img src="https://img.shields.io/badge/macOS-universal-000000?logo=apple" alt="macOS universal">
  <img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="Apache-2.0">
</p>

Mnelyra 管两条连接方向。ChatGPT 和其他 MCP 客户端可以通过远程 MCP 地址进入当前 Workspace，直接读取和修改文件、搜索项目、应用 Patch、执行命令和测试、检查 Git、查看图片。原生 Codex 则可以通过本机 Responses bridge 使用当前 ChatGPT 账号可用的 Web 推理档位。

两条链路彼此独立。远程 MCP 只服从 Mnelyra 当前 Workspace 和权限设置；Web Models 只改变 Codex 的模型传输，Codex 仍使用自己的当前工作区、工具、审批和 sandbox。Mnelyra 的 Workspace 记忆继续跟 Mnelyra 项目走。

![Mnelyra 通用设置](static/readme/mnelyra-general.png)

> README 截图仅使用演示数据。

## Mnelyra 实际解决什么

客户端只需要配置一次 Mnelyra。Cloudflare 或 FRP 负责把当前本地 MCP 服务提供给远端客户端；切换 Workspace 只改变项目根目录，不会让 ChatGPT 跟着换地址、重新配一遍连接。

ChatGPT Web 也可以成为本地项目客户端。连接 Mnelyra 的自定义 MCP app 后，网页里的对话可以调用文件、Patch、命令、测试、Git、图片和 history 工具，操作范围由当前 Workspace 与 Mnelyra 权限设置决定。

另一边，Mnelyra 可以把原生 Codex 接到当前登录的 ChatGPT Web 会话。Codex 仍然使用原生的 `gpt-5.6-sol` 模型入口；Low、Medium、High 分别映射到 ChatGPT Web 对应的推理档位。

上下文设置也放在 Mnelyra 里。**自动**模式清除旧 bridge 写入的固定上下文与总结阈值，由 Codex 使用当前模型默认值和自己的压缩机制；**1M** 会写入 `1,000,000` 上下文和 `900,000` 自动总结阈值；**自定义**可以直接填写两个值。这样不需要再手改 Codex 配置文件。

## 工作方式

![Mnelyra 架构](static/readme/mnelyra-architecture.svg)

客户端连接和项目根目录彼此分开。多个项目只需要导入一次，之后从侧栏切换 Workspace；客户端仍然使用原来的 Mnelyra 连接。

## 快速开始

### 1. 安装

从 [GitHub Releases](https://github.com/biaroli/Mnelyra/releases/latest) 下载当前版本。Release 客户端会在启动时检查 GitHub Releases；发现新版本后可直接在应用内完成签名验证、下载和更新。

| 系统 | 安装包 |
| --- | --- |
| Windows 10/11 x64 | `Mnelyra_*_x64-setup.exe` |
| macOS Intel + Apple Silicon | `Mnelyra_*_universal.dmg` |

macOS 当前没有 Apple Developer notarization，首次打开时可能需要在 系统设置 → 隐私与安全性 中确认。

### 2. 添加工作区

点击侧栏的文件夹按钮，选择项目根目录。选择工作区后，Mnelyra 会把 MCP 根目录切到该项目并激活它，不需要第二个启动按钮。

### 3. 选择连接方式

打开 **连接**：

| 场景 | 连接方式 |
| --- | --- |
| 固定公网域名 | Cloudflare Named Tunnel |
| 临时测试地址 | Cloudflare Quick Tunnel |
| 自托管反向代理 | FRP |
| 在原生 Codex 中使用 ChatGPT Web 推理 | 网页模型接入 |

本机 MCP 默认地址类似：

```text
http://127.0.0.1:28766/mcp
```

### 4. 配置认证

打开 **设置 → 认证**。公网 MCP 推荐 OAuth；也可以使用 Bearer Token。使用 OAuth 时，在浏览器授权页输入 Mnelyra 显示的授权码即可。

把 Mnelyra 显示的 `/mcp` 地址填入支持自定义 MCP 的客户端即可。第一次连接可以检查：

```text
server_info
get_default_cwd
git_status
```

`get_default_cwd` 应该指向当前选择的 Workspace，`git_status` 应该读取同一个项目。

## 把 Mnelyra 接到 ChatGPT 网页版

ChatGPT 不能直接连接 `127.0.0.1` 上的 MCP。先在 Mnelyra 的 **连接** 页面配置 Cloudflare 或 FRP，拿到可从外部访问的 `/mcp` 地址。

在 ChatGPT 中启用 **Developer mode**，然后创建自定义 app，并把它连接到 Mnelyra 的 `/mcp` 地址。

进入 **Apps → Create**，名称填 `Mnelyra`，MCP Server URL 填 Mnelyra 提供的远程 `/mcp` 地址，并选择对应认证方式。点击 **Scan Tools**。如果使用 OAuth，浏览器会进入授权流程；按提示完成授权后等待工具扫描结束，再点击 **Create**。

创建完成后，在聊天输入框的 **+ → More** 中选择 Mnelyra，就可以从 ChatGPT 调用当前 Workspace 的工具。以后 Mnelyra 增删了 MCP 工具，需要回到 **Settings → Apps** 刷新这个 app，让 ChatGPT 重新读取工具列表。

![Mnelyra 认证页](static/readme/mnelyra-authentication.png)

OpenAI 的当前说明见 [Developer mode and MCP apps in ChatGPT](https://help.openai.com/en/articles/12584461-developer-mode-and-mcp-apps-in-chatgpt) 和 [Apps in ChatGPT](https://help.openai.com/en/articles/11487775-apps-in-chatgpt)。

## 在 Codex 中使用 ChatGPT Web 推理

打开 **连接 → 网页模型接入**，点击 **启动**。第一次使用时，Mnelyra 会打开自己管理的 ChatGPT 窗口让你完成登录；登录完成后，这个会话由 Mnelyra 单独维护，不会占用你平常使用的浏览器窗口。

桥接不会新增一组自定义模型。Codex 仍然选择原生的 `GPT-5.6 Sol`，只需要照常切换推理档位：

| Codex 推理档位 | ChatGPT Web 档位 |
| --- | --- |
| Low | Low / Instant |
| Medium | Medium |
| High | High |

启动桥接后，**新建一个 Codex 对话**，选择 `GPT-5.6 Sol` 和需要的推理档位即可。已经加载的旧对话会继续沿用创建时的通道，所以切换原生/Web 路由时，新建对话最干净。

模型推理由 ChatGPT Web 完成时，Codex 仍负责本地工具执行、审批、sandbox 和 MCP 工具；工具结果会回到网页模型继续下一轮，因此正常的 Codex 工具工作流可以继续使用。

需要恢复原生 Codex 时，在 Mnelyra 中点击 **断开**。Mnelyra 会还原之前的 Codex 配置，并在切换过程中避免已经加载的旧对话直接失去连接。Web Models 不需要 OpenAI Tunnel ID，也不需要模型 API Key。

网页模型能力取决于当前登录的 ChatGPT 账号以及 ChatGPT Web 本身，因此可用性可能随账号能力或网页 UI 更新而变化。

### Codex 上下文与自动总结

在 **设置 → 通用 → 开发者模式** 中可以配置 Codex 的 `model_context_window` 与 `model_auto_compact_token_limit`。**自动**是推荐默认值：Mnelyra 清除两个覆盖项，把上下文和压缩交给 Codex。需要固定大窗口时可以选择 **1M**，或用 **自定义**明确填写上下文和自动总结阈值。最终有效上限仍由当前模型和通道决定。

## 工作区记忆

Mnelyra 的记忆跟 Workspace 走。不同上游可以围绕同一个项目继续工作，记忆不绑定某一个 ChatGPT 会话。

记忆页面显示当前焦点、最近变化、未完成项和可恢复 checkpoint。跨客户端连续性由持久 history 档案和下面的 history 工具提供。

公开的 history 工具包括：

| 工具 | 用途 |
| --- | --- |
| `history_session_bootstrap` | 新对话初始化或恢复项目历史 |
| `history_session_checkpoint` | 保存本轮决策、改动、验证和下一步 |
| `history_session_search` | 搜索旧会话 |
| `history_session_read` | 读取指定历史档案 |
| `history_session_validate` | 检查历史编号和索引 |

如果 provider 本身提供可恢复 checkpoint，Mnelyra 也可以把它显示在工作区记忆中；没有 checkpoint 时不会制造空占位。

## 权限

开发者模式提供应用级权限总阀门：

| 模式 | 行为 |
| --- | --- |
| **自动** | 不额外收紧，沿用当前下游策略 |
| **只读** | 顶层阻止写入、Patch 和命令执行等可变更操作 |
| **工作区读写** | 允许 Workspace 内读写和网络访问；文件操作仍以当前 Workspace 为边界 |

Windows 下的 OpenAI 编码链路保留 workspace sandbox 与 MiKTeX 兼容配置；这不等于任意宿主机文件系统访问。

## 公网路由

### Cloudflare

Named Tunnel 适合长期固定地址；Quick Tunnel 适合临时测试。使用 Named Tunnel 时，先在 Cloudflare 中绑定准备使用的域名或子域名，再把该 hostname 填入 Mnelyra。固定地址示例：

```text
https://mcp.example.com/mcp
```

### FRP

已有 FRPS 服务端时，在连接页填写服务器、端口、子域名和 Token。Mnelyra 只维护当前应用级 FRP 路由，不需要为每个工作区重复建配置。

## 从源码运行

需要 Node.js、npm、Rust stable，以及 Tauri 2 对应平台依赖。

```bash
npm ci
npm run desktop
```

提交前检查：

```bash
npm run check
npm run build

cd src-tauri
cargo fmt -- --check
cargo test
cargo clippy --all-targets -- -D warnings
```

维护者说明见 [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)。

## 安全

Mnelyra 可以修改文件并执行命令。公网 endpoint 应启用认证，只连接自己控制或明确可信的客户端。

Workspace 边界、权限总阀门和下游 sandbox 是不同层级的限制。Mnelyra 不应被当成完整的操作系统隔离容器。

ChatGPT Web bridge 属于非官方浏览器集成，网页 UI 或账号能力变化可能影响它。请只使用自己的账号，并遵守对应平台的服务条款和 Workspace 策略。

## License

Mnelyra 使用 [Apache License 2.0](LICENSE)。

## 鸣谢

感谢 [mybolide/coding-tools-mcp](https://github.com/mybolide/coding-tools-mcp)、[xyTom/coding-tools-mcp](https://github.com/xyTom/coding-tools-mcp)、[Tauri](https://github.com/tauri-apps/tauri)、[Svelte](https://github.com/sveltejs/svelte) 及其贡献者。Copyright 2026 Coding Tools MCP Contributors。

Mnelyra 与 OpenAI、Anthropic、Cloudflare 没有隶属或官方合作关系。
