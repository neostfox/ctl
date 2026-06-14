<div align="center">

# ctl —— AI Dev Control Plane

**用 Rust 打造的确定性 AI 开发控制层：边界优先的任务生命周期、治理与可审计验收闸门。**

[![release](https://img.shields.io/github/v/release/neostfox/ctl?sort=semver)](https://github.com/neostfox/ctl/releases)
[![build](https://github.com/neostfox/ctl/actions/workflows/release.yml/badge.svg)](https://github.com/neostfox/ctl/actions/workflows/release.yml)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](#许可证)
![platforms](https://img.shields.io/badge/platform-linux%20%7C%20macOS%20%7C%20windows-lightgrey)

</div>

> 一句话定位：**Trellis 的任务/归档/上下文能力 + Spec-Driven 的闭环控制论，用 Rust 做成确定性控制层**，让 AI 编码工具（OMP / Claude Code / Codex …）只能在「可定义、可检查、可审计」的边界内工作。

`ctl` 不是又一个代码生成器。它解决的是 AI 协作里真正难的那部分：

```text
不是「让 agent 做更多事」，
而是「明确什么能做、谁能做、在什么范围内做、越界谁来决定」。
```

事件溯源的任务账本 + 机器可执行的写入边界 + 确定性验收闸门，三者组成一个不靠自觉、靠机制的护栏。

---

## 为什么用 ctl？

| 维度 | 没有 ctl | 有了 ctl |
|---|---|---|
| **写入边界** | agent 可以改任何文件 | 每个任务声明 `write_allow`；越界写入由 hook **fail-closed 拦截** |
| **事实源** | 散落的 Markdown 进度 | `events.jsonl` 追加式唯一事实源，`task.json` 由 replay 重建 |
| **验收** | 「我觉得做完了」 | 机器可执行 gate（`cargo_check/test/fmt/clippy` …）通过才能推进状态 |
| **可审计** | 全靠聊天记录回溯 | 每次状态变化都是一条带 hash / 来源 / 身份的 canonical event |
| **多工具治理** | 每个 CLI 一套规则 | 单一来源治理，OMP + Claude Code 共用同一套边界 |

---

## 前置要求

- **运行二进制**：无需任何运行时，下载即用（见下方安装）。
- **从源码构建**（可选）：Rust ≥ 1.74（stable）。

支持平台：Linux（x64 / arm64）、macOS（Intel / Apple Silicon）、Windows（x64，原生，无需 WSL）。

---

## 快速开始

### 1. 安装

**Linux / macOS（bash）**

```bash
curl -fsSL https://raw.githubusercontent.com/neostfox/ctl/master/scripts/install.sh | sh
```

**Windows（PowerShell）**

```powershell
irm https://raw.githubusercontent.com/neostfox/ctl/master/scripts/install.ps1 | iex
```

安装脚本会自动识别系统/架构，从 [GitHub Releases](https://github.com/neostfox/ctl/releases) 下载对应二进制、校验 SHA256，并装入 PATH。

<details>
<summary>可选项与其他安装方式</summary>

安装指定版本或自定义目录：

```bash
# bash
curl -fsSL https://raw.githubusercontent.com/neostfox/ctl/master/scripts/install.sh | sh -s -- --version v0.0.1 --dir ~/.local/bin
# 或用环境变量
CTL_VERSION=v0.0.1 CTL_INSTALL_DIR=~/.local/bin sh install.sh
```

```powershell
# PowerShell
$env:CTL_VERSION="v0.0.1"; irm https://raw.githubusercontent.com/neostfox/ctl/master/scripts/install.ps1 | iex
```

**从源码构建：**

```bash
cargo build --release        # 产物：target/release/ctl
```

**手动下载：** 直接到 [Releases](https://github.com/neostfox/ctl/releases) 取对应平台的 `ctl-<target>.tar.gz` / `.zip`。

</details>

### 2. 初始化

在你的项目根目录：

```bash
ctl init                     # 创建 .ctl/ 任务账本，注入治理 hook
```

### 3. 跑一个受控任务

```bash
ctl task create --id 06-14-fix-login \
  --objective "修复登录态过期" \
  --read-scope src --write-allow src/auth --gates cargo_test
ctl task ready  --id 06-14-fix-login
ctl task start  --id 06-14-fix-login   # 进入 in_progress，写入边界开始生效
# …agent 在 src/auth 内实现…
ctl gate run --id 06-14-fix-login --gate cargo_test   # 跑验收闸门，记录 evidence
ctl task submit --id 06-14-fix-login
ctl task finish --id 06-14-fix-login
ctl task archive --id 06-14-fix-login
```

---

## 如何使用

日常协作下你几乎不用手敲命令——AI agent 通过内置 skill 驱动整个循环，你只在关键节点确认：

1. **描述需求**：直接对 agent 说要做什么。
2. **确认边界**：agent 推断 objective / scope / gates，给出任务提案，你回答 `yes / 调整 / skip`。
3. **受控实现**：agent 只能在 `write_allow` 内写入；越界写入被 hook 当场拦截。
4. **验收与完成**：agent 自动跑 gate，控制层独立检查完成条件后，你确认归档。

> CLI 是底层能力；治理与编排由 [`.omp/`](./.omp)（OMP）与 [`.claude/hooks/`](./.claude)（Claude Code）里的 hook 强制执行。

---

## 工作原理

`ctl` 把工程控制论落到具体对象上——目标状态、观测、比较、控制动作，全部可机器表达：

```text
        ┌─────────── 控制闭环 ───────────┐
proposal → approval → scoped lease → implement → audit_hold
                                                     │
                                  deterministic audit ▼
                                human_resume | completed | stopped
```

四条硬约束撑起整个系统：

- **事件溯源**：`events.jsonl` 是唯一事实源（append-only）；`task.json` / `control.json` 都是 replay 出来的投影。外部只能提交 evidence，由控制层验证后才生成 canonical event。
- **写入边界 fail-closed**：每个任务声明允许读写的路径；`ctl` 不可用时 hook **拦截写操作而非放行**——不可执行的边界绝不静默放行。
- **确定性验收**：gate 是固定模板（无任意 shell），通过才允许 `in_progress → review → completed`。
- **审计是权限状态，不是提示**：命中 schema / scope / 受保护路径 / 批量变更触发器后立即进入只读 `audit_hold`，由控制层独立核对，而非靠实现者自述「我做完了」。

> 完整的控制论映射、drift 计算、子智能体调度协议、schema 设计等详见 [DESIGN.md](./DESIGN.md)。

---

## 命令一览

```text
ctl init                              初始化 .ctl/ 账本与治理 hook
ctl task create|ready|start|submit|finish|archive|status
                                      任务生命周期
ctl context build                     生成 implement / check 上下文 manifest
ctl gate run|record                   执行 / 记录验收闸门
ctl assignment create|export          导出结构化任务包（manual adapter）
ctl run ingest                        回填执行结果为 evidence
ctl boundary check                    校验某次写入是否越界
ctl replay | reconcile | validate     重建投影 / 校验事件流
ctl audit | report | board            审计报告 / 任务总览 / 跨任务控制板
ctl architecture check                架构合规（依赖方向、domain 纯度）
ctl doctor                            诊断账本健康
```

完整子命令见 `ctl --help`。

---

## 资源

| 文档 | 内容 |
|---|---|
| [DESIGN.md](./DESIGN.md) | 设计与愿景：控制论闭环、drift、子智能体、schema 冻结 |
| [ROADMAP.md](./ROADMAP.md) | 里程碑 M0–M6+ 与退出条件 |
| [ARCHITECTURE_GUARDRAILS.md](./ARCHITECTURE_GUARDRAILS.md) | 必须遵守的架构约束 |
| [GLOB_WORKFLOW.md](./GLOB_WORKFLOW.md) | 用 OMP `/glob` 分阶段推进实现 |
| [AGENTS.md](./AGENTS.md) | 给 AI agent 的项目说明 |

---

## FAQ

**和 Trellis / Spec-Kit 有什么不同？**
它们以 Markdown 流程为主；`ctl` 把任务状态、事件、验收做成 Rust 确定性状态机，边界由机器强制，而不是约定俗成。

**支持哪些 AI 工具？**
当前激活 OMP（原生 hook）与 Claude Code（PreToolUse hook）。控制层只依赖统一协议，Codex / OpenCode 为规划中的兼容目标。

**会不会限制太死？**
默认只读、最小权限是刻意设计。需要越界时走 `ctl apply` 申请受审批的路径例外，而不是直接放开。

**需要联网 / 接模型吗？**
不需要。`ctl` 本身不调用模型，只做状态、边界与验收；模型由你选用的执行器提供。

---

## 许可证

MIT。本项目用 Rust 重新实现状态机与协议，只吸收 Trellis / Spec-Driven 的思想，不复制其代码。
