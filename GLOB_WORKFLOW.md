# `/glob` 项目推进指南

本文档用于指导后续使用 OMP `/glob` 批量推进 AI Dev Control Plane。它是参考流程，不是自动授权。每次 `/glob` 仍然必须声明里程碑、目标、允许路径、禁止路径、不变量和验收命令。

## 总原则

不要一次性执行：

```text
/glob 完成整个项目
```

正确方式是按里程碑推进：

```text
M0 收尾
  -> M1 本地任务账本
  -> M2 边界 / gate / 归档
  -> M3 Manual 闭环 MVP
  -> M3 dogfood
  -> M4 OMP 单执行器
  -> M4 dogfood
  -> M5 drift 控制闭环
  -> M6 受限多智能体
```

每个 `/glob` 必须包含：

```text
1. 里程碑
2. 目标
3. 允许修改的路径
4. 禁止修改的路径
5. 必须遵守的不变量
6. 明确不做的内容
7. 验收命令
8. 期望输出
```

核心约束不随 `/glob` 改变：

```text
events.jsonl 是唯一 canonical truth
task.json / control.json 是可重建投影
telemetry / agent output / reviewer output 都只是 evidence
domain/ 保持纯函数，不访问文件系统、Git、进程、网络或当前时间
completion interlock 由控制层独立执行
模型、reviewer、telemetry 不能自行宣布完成
drift 只能触发暂停、解释或重规划 proposal，不能自动扩权
```

---

## Step 0：M0 收尾与验证

```text
/glob "
里程碑：M0 boundary protocol freeze 收尾

目标：
完成 M0 当前代码和文档的收尾，使项目处于可继续进入 M1 的干净状态。

允许修改：
- src/cli/mod.rs
- .gitignore
- Cargo.lock
- README.md
- ROADMAP.md
- ARCHITECTURE_GUARDRAILS.md

禁止修改：
- schemas/**
- src/domain/** 的业务语义
- fixtures/**
- tests/**

任务：
1. 修复 src/cli/mod.rs 中已知 unused PathBuf import。
2. 检查 .gitignore 是否覆盖 target/、本地临时文件、编辑器缓存。
3. 确认 Cargo.lock 是否应保留并提交。
4. 不新增依赖。
5. 不新增 schema。
6. 不实现 M1/M2/M3 功能。

验收：
- cargo fmt --check
- cargo check --locked --offline
- cargo test --locked --offline
- cargo clippy --locked --offline -- -D warnings
- cargo run --locked --offline -- architecture check

输出：
- 修改文件列表
- 验证命令结果
- 是否可以进入 M1
"
```

---

## Step 1：M1 本地任务账本

```text
/glob "
里程碑：M1 local task ledger

目标：
实现本地任务账本，使 events.jsonl 成为唯一事实源，task.json 由 replay 生成。

允许修改：
- src/domain/**
- src/infrastructure/store/**
- src/application/**
- src/cli/mod.rs
- fixtures/**
- tests/**

禁止修改：
- schemas/**
- Cargo.toml
- Cargo.lock
- adapters/**
- infrastructure/gates/**
- infrastructure/boundary/**，除非仅接已有接口
- README.md
- ROADMAP.md
- ARCHITECTURE_GUARDRAILS.md

必须遵守：
1. events.jsonl 是唯一 canonical truth。
2. task.json 只能由 replay 生成。
3. domain/ 保持纯函数，不访问文件系统、Git、进程、网络、当前时间。
4. 每条事件必须有严格递增 seq。
5. command_id 必须幂等。
6. task_id 必须绑定一致。
7. 损坏、重复、乱序事件必须拒绝，不静默修复。
8. M1 不实现 gate runner、adapter、telemetry drift、scheduler、OMP runtime。

需要实现的命令：
- ctl init
- ctl task create
- ctl task revise
- ctl task ready
- ctl task start
- ctl task status
- ctl task cancel
- ctl replay
- ctl validate
- ctl doctor

验收：
- 临时目录内可以 create -> ready -> start -> cancel。
- 删除 task.json 后可以仅依赖 events.jsonl 恢复。
- 重复 replay 输出一致。
- 损坏事件被拒绝。
- cargo fmt --check
- cargo check --locked --offline
- cargo test --locked --offline
- cargo clippy --locked --offline -- -D warnings
- cargo run --locked --offline -- architecture check

输出：
- 已实现命令
- 状态机覆盖情况
- 事件持久化策略
- 未进入 M2 的内容
"
```

---

## Step 2：M2 边界、Gate 与归档

```text
/glob "
里程碑：M2 boundary gates archive

目标：
把任务 scope、真实工作区 diff、required gates 和归档接入任务生命周期。

允许修改：
- src/domain/**
- src/infrastructure/boundary/**
- src/infrastructure/gates/**
- src/infrastructure/store/**
- src/application/**
- src/cli/mod.rs
- fixtures/**
- tests/**

禁止修改：
- schemas/**，除非先输出单独 schema 变更计划
- Cargo.toml
- Cargo.lock
- adapters/**
- OMP 相关代码
- 多智能体调度代码

必须遵守：
1. finish 必须检查 scope、required gates、hold、baseline。
2. gate 未通过不能 finish。
3. agent 自述或 telemetry 不能直接完成任务。
4. 越界 diff 必须记录 boundary violation 并进入 hold。
5. .git/**、canonical events、策略文件、gate 定义默认禁止写入。
6. gate runner 只能使用明确模板，不能开放任意 shell。
7. M2 不实现自动 agent、不实现 OMP adapter、不实现 drift 自动化。

需要实现的命令：
- ctl context build
- ctl boundary check
- ctl boundary explain
- ctl gate run
- ctl gate record
- ctl task submit
- ctl task reopen
- ctl task finish
- ctl task archive
- ctl reconcile

验收：
- 越界修改能被检测并阻止 finish。
- gate 失败无法 finish。
- required gate 最新 PASS 后才可完成。
- 归档终态任务具备幂等语义。
- cargo fmt --check
- cargo check --locked --offline
- cargo test --locked --offline
- cargo clippy --locked --offline -- -D warnings
- cargo run --locked --offline -- architecture check

输出：
- boundary check 规则列表
- gate runner 模板列表
- completion interlock 覆盖情况
- 仍未实现的 M3/M4/M5/M6 内容
"
```

---

## Step 3：M3 Manual 闭环 MVP

```text
/glob "
里程碑：M3 manual closed-loop MVP

目标：
不接任何模型，先用 manual adapter 跑通 assignment export -> 人/AI 外部执行 -> agent-output ingest -> audit -> report -> finish/archive 的闭环。

允许修改：
- src/domain/**
- src/application/**
- src/adapters/manual/**
- src/infrastructure/store/**
- src/infrastructure/boundary/**
- src/infrastructure/gates/**
- src/cli/mod.rs
- fixtures/**
- tests/**

禁止修改：
- OMP adapter
- Codex/Claude/OpenCode adapter
- scheduler
- 多智能体
- drift runtime
- daemon/database/network service
- Cargo.toml
- Cargo.lock，除非必要并单独说明

必须遵守：
1. manual 是第一个正式 adapter，不是临时兜底。
2. assignment.json 是 adapter contract。
3. agent-output.json 是不可信 evidence。
4. 控制层必须独立检查 diff、gate、hash、scope。
5. reviewer PASS、agent PASS、telemetry PASS 都不能直接完成任务。
6. assignment.json / agent-output.json 作为后续 OMP adapter contract baseline。

需要实现的命令：
- ctl assignment create
- ctl assignment export
- ctl run ingest --adapter manual
- ctl audit
- ctl report

验收：
- 从 task create 到 assignment export 再到 output ingest 可端到端完成。
- evidence 包含 input/output/command/file hash。
- 重放后 audit 结论一致。
- 中断后 control status 可恢复上下文。
- 至少提供一个完整 fixture 覆盖 small task。
- cargo fmt --check
- cargo check --locked --offline
- cargo test --locked --offline
- cargo clippy --locked --offline -- -D warnings
- cargo run --locked --offline -- architecture check

输出：
- manual adapter contract
- assignment 示例
- agent-output 示例
- audit report 示例
- 是否满足 M3 MVP
"
```

---

## Step 4：M3 Dogfood 真实任务

M3 完成后不要马上进 M4。先用它治理真实小任务。

```text
/glob "
里程碑：M3 dogfood

目标：
使用当前 control CLI 治理至少 3 个真实 small task，发现协议缺口。

允许修改：
- README.md
- ROADMAP.md
- ARCHITECTURE_GUARDRAILS.md
- fixtures/**
- tests/**
- src/** 中仅限修复 dogfood 暴露的 M3 bug

禁止修改：
- OMP adapter
- scheduler
- drift runtime
- 多智能体
- 新依赖

任务：
1. 选择 3 个 small task。
2. 每个任务必须经过 create -> ready -> start -> assignment export -> output ingest -> audit -> finish/archive。
3. 记录哪些字段冗余、哪些字段缺失、哪些 gate 不可执行。
4. 修复 M3 范围内的 bug。
5. 不引入后续里程碑能力。

验收：
- 3 个 dogfood task 全部有 evidence。
- 每个 task 可 replay。
- audit report 可解释完成/暂停原因。
- cargo test --locked --offline
- cargo run --locked --offline -- architecture check

输出：
- dogfood 报告
- M4 是否可以开始的判断
- M3 遗留问题清单
"
```

---

## Step 5：M4 OMP 单执行器隔离运行

```text
/glob "
里程碑：M4 OMP single executor adapter

目标：
让 OMP 作为单执行器消费 assignment，并且只能在 disposable worktree 中写入；主工作区只接受经过 apply gate 的 diff。

允许修改：
- src/adapters/omp/**
- src/application/**
- src/infrastructure/store/**
- src/infrastructure/boundary/**
- src/infrastructure/gates/**
- src/cli/mod.rs
- fixtures/**
- tests/**

禁止修改：
- 多智能体 scheduler
- drift runtime
- daemon/database/network service
- 自动 merge/commit/push/deploy
- 长期密钥注入
- agent 直接写主工作区

必须遵守：
1. OMP 只能执行单个 assignment。
2. OMP 写入 disposable worktree。
3. worktree diff 必须经过 apply gate。
4. 越界 diff 永远不能 apply。
5. 删除、依赖变化、Git 操作、网络访问、公共 API、安全策略变化需要 step-up approval。
6. approval 只能批准结构化 request。
7. lease 绑定 task、run、资源、动作、TTL、max_uses。
8. 自动 reviewer 只能提交 evidence，completion interlock 仍由控制层执行。

需要实现的命令：
- ctl adapter capabilities omp
- ctl workspace create
- ctl workspace diff
- ctl workspace apply
- ctl run start --adapter omp
- ctl approval request
- ctl approval grant
- ctl approval deny

验收：
- OMP 中断后可以恢复或明确失败。
- 越界 diff 被拒绝。
- 过期 lease 被拒绝。
- 跨 task lease 被拒绝。
- 高风险变更未经 approval 不能 apply。
- cargo fmt --check
- cargo check --locked --offline
- cargo test --locked --offline
- cargo clippy --locked --offline -- -D warnings
- cargo run --locked --offline -- architecture check

输出：
- OMP adapter contract
- worktree apply gate 说明
- lease failure cases
- M4 dogfood 建议
"
```

---

## Step 6：M5 可解释 Drift 控制闭环

```text
/glob "
里程碑：M5 explainable control loop

目标：
实现 telemetry ingest、drift compute、drift explain、next-action，使系统能回答为什么继续、暂停、重规划或请求 rescope。

允许修改：
- src/domain/**
- src/application/**
- src/infrastructure/store/**
- src/cli/mod.rs
- fixtures/**
- tests/**

禁止修改：
- scheduler
- 多智能体并发
- 自动重规划执行
- 模型评分
- dashboard
- OpenTelemetry Collector
- 新增网络服务

必须遵守：
1. drift 使用透明规则，不使用模型自由打分。
2. 相同 evidence 和规则必须生成相同 decision。
3. 未知信号不能放宽权限。
4. drift 升高只能触发 annotate、replan proposal、rescope request、stop。
5. replan/rescope 只生成结构化 proposal，不自动修改 scope、不批准 lease、不启动执行。
6. SDD 的 effort_delta、super_delta、unplanned_deps 只能作为 evidence signal。

需要实现的命令：
- control telemetry add
- control drift compute
- control drift explain
- control next-action

验收：
- Golden fixtures 对应固定动作。
- 每次 decision 输出 signals、rule IDs、evidence IDs。
- control.json 可由 reconcile 重建。
- drift 不会自动扩权。
- cargo fmt --check
- cargo check --locked --offline
- cargo test --locked --offline
- cargo clippy --locked --offline -- -D warnings
- cargo run --locked --offline -- architecture check

输出：
- drift signal 表
- drift threshold 表
- next-action 示例
- replan proposal 示例
"
```

---

## Step 7：M6 受限多智能体

```text
/glob "
里程碑：M6 restricted multi-agent scheduling

目标：
在写入 scope 不重叠的前提下，让多个 agent 安全并行执行，提高吞吐量。

允许修改：
- src/domain/**
- src/application/**
- src/adapters/**
- src/infrastructure/store/**
- src/infrastructure/boundary/**
- src/infrastructure/gates/**
- src/cli/mod.rs
- fixtures/**
- tests/**

禁止修改：
- 自动 merge
- 自动 commit/push/deploy
- 动态扩权
- 多 agent 写同一文件
- 长期密钥
- 数据库/daemon，除非另行设计

必须遵守：
1. 这时才允许拆出 AgentRun aggregate。
2. 每个写入 agent 使用独立 worktree。
3. 每个写入 agent 必须有独立 scoped lease。
4. 写入 scope 重叠必须拒绝。
5. 只读 agent 可以并发。
6. 合并候选仍需人工确认。
7. 崩溃恢复不能重复执行副作用。
8. 共享 .git 风险必须有明确防护。

需要实现的命令：
- ctl schedule plan
- ctl schedule validate
- ctl schedule run
- control agent report
- ctl workspace merge-candidate

验收：
- 重叠写入被拒绝。
- 非重叠写入可以并发。
- 每个 AgentRun 可恢复或明确失败。
- 合并候选需人工确认。
- dirty worktree 和 merge conflict 可恢复。
- cargo fmt --check
- cargo check --locked --offline
- cargo test --locked --offline
- cargo clippy --locked --offline -- -D warnings
- cargo run --locked --offline -- architecture check

输出：
- schedule plan 示例
- overlap detection 规则
- AgentRun 状态机
- merge-candidate 报告格式
"
```

---

## 每批完成后必须检查的问题

```text
1. 本批是否引入了下一里程碑能力？
2. 是否新增依赖？
3. 是否新增 schema？
4. 是否新增副作用类型？
5. 是否存在 agent / reviewer / telemetry 直接完成任务的路径？
6. 是否存在 drift 自动扩权？
7. 是否所有状态都能从 events.jsonl replay？
8. 是否所有 projection 都可删除重建？
9. 是否所有 gate 失败都能停在可恢复状态？
```

## 什么时候停止推进

遇到以下情况，不要继续下一批 `/glob`：

```text
cargo test 失败
architecture check 失败
task.json 不能 replay
gate 失败仍可 finish
agent output 可直接改变 canonical state
drift 可自动扩大 scope
adapter 可绕过 assignment / lease
OMP 可直接写主工作区
多 agent 写 scope 无法证明不重叠
```

## 最重要的限制

即使不依赖项目级 hook，也不要让 `/glob` 变成无限制自动实现器。

这个项目的核心不是“让 agent 做更多”，而是：

```text
让 agent 的每一步都能被声明、验证、审计、暂停、恢复。
```
