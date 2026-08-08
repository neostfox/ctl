# ctl Agent 操作指南（面向 AI Agent）

> 这份文档是给 **你**——驱动 ctl 的 AI agent（Claude Code / OMP / opencode）——的操作指南：从头到尾怎么用 ctl 干活、哪些地方必须停下等人、哪些红线绝不能碰。
>
> 你需要先知道的**项目身份与架构**（ctl 是什么、依赖方向、禁项）在 [AGENTS.md](./AGENTS.md)；**每条命令的精确语义**在 [USAGE.md](./USAGE.md) 与 `ctl --help`。本文专讲**操作层的工作流与契约**。

---

## 目录

- [ctl 对你意味着什么（契约）](#ctl-对你意味着什么契约)
- [两层治理模型](#两层治理模型)
- [何时创建任务 vs 跳过](#何时创建任务-vs-跳过)
- [治理管线（proposal-first）](#治理管线proposal-first)
- [任务生命周期：你的命令流](#任务生命周期你的命令流)
- [工作流技能（phase map）](#工作流技能phase-map)
- [边界与 hook：你必须知道的](#边界与-hook你必须知道的)
- [认识状态层（record-and-disclose）](#认识状态层record-and-disclose)
- [HITL 节点：你必须停下等人的地方](#hitl-节点你必须停下等人的地方)
- [red→green TDD（--tdd）](#redgreen-tdd---tdd)
- [交接与上下文压缩](#交接与上下文压缩)
- [一个完整的 agent 驱动示例](#一个完整的-agent-驱动示例)
- [反模式（禁止）](#反模式禁止)

---

## ctl 对你意味着什么（契约）

你和 ctl 的分工是**不可协商的**（来自 [`.agent/protocols/workflow-skills.md`](./.agent/protocols/workflow-skills.md)）：

| 你（skill + agent）管 | ctl 管 |
|---|---|
| **语义工作流**：想什么、按什么顺序、每阶段产出什么 artifact | **事实、边界、证据、闸门、账本、诚实披露** |

三条铁律：

1. **你提交 evidence，控制层生成 canonical event。** 你**绝不**直接写 `events.jsonl` 或 `task.json`——前者是 append-only 唯一事实源，后者是 replay 投影。外部角色不能追加 canonical event。
2. **skill 永不放松边界、永不声明完成、永不用自己的判断替代 ctl evidence。** 工作流纪律**不是证明**：它不替代 gate / audit / reviewer 独立性 / 防篡改证据，也从不产出 verdict。
3. **「完成」= 控制层能看到的证据 artifact，不是你的断言。** 永远问自己：「证据在哪？」（build/test/run 输出，不是「我觉得做完了」。）

---

## 两层治理模型

```
ctl task（parent）  — 声明的 scope / gates / boundaries（ctl 账本）
  └─ host subtasks  — 你运行时自带的 step/todo 跟踪，永远在 parent 的 write_allow 内
```

你**主动**在承担风险的工作**之前**创建 parent ctl 任务，再用宿主原生机制拆成 subtasks。宿主的 ctl gate 跑在 **observe mode**：越界或无任务的变更**被允许但被记录**到 `.ctl/decisions.jsonl`，并给你一条可见的 warning。**warning 是提示你去创建/加宽任务，不是继续无治理的许可。** 即便在 observe mode，硬核仍拒绝：受保护路径、依赖变更（需 step-up）、held 任务、跨任务写重叠。`ctl` 不可用时，路径作用域的写工具 **fail-closed（拦截）** 直到它恢复。

---

## 何时创建任务 vs 跳过

**主动创建 ctl 任务**，当你检测到：

- 多文件改动（要改 2+ 文件）；
- 有明确目标的 feature / bugfix / refactor；
- 一个**既需要调查又需要改码**的问题；
- 任何受益于审计轨迹与边界强制的工作。

**跳过治理、自由工作**，当：

- 纯对话 / 问答；
- 只读探索；
- 琐碎的单文件编辑（typo、注释）；
- 用户明确说「skip control」。

> 琐碎改动可以直接改（gate 会以 observe mode 记录这次无任务写入），或用一个 `ctl task quick` 留痕。

---

## 治理管线（proposal-first）

非琐碎工作都走这条管线（每一步由对应 skill 驱动；routing 时**报告当前 station 和它的 artifact**，让人始终知道管线停在哪）：

```
triage（本协议）
  → align（grill / ctl-grill-with-spec）   第一性原理对齐 + micro-decision 面谈
  → PRD（ctl-to-prd）                        管线第一个硬检查点（多任务时）
  → tasks（ctl-to-tasks）                    垂直切片
  → implement（control-guard，--tdd 可选）
  → wrap-up（finish → ctl-spec）
```

- **琐碎**（typo、单文件明显修复）——跳过管线，直接改或用 quick 任务。
- **其他一切**——`ctl task create` **之前**先跑 align（grill）：第一性原理提案 + micro-decision 面谈（一次一问，每问带推荐答案；事实来自 repo，方向来自用户）。**用户确认前不要构建。**
- **多个持久任务** → 确认后的对齐先过 **PRD** 再过 **tasks**。
- 一个「靠产出**证据而非代码**来回答」的问题 → **research/spike** 任务（`--kind research`，以证据 + 不确定性了结完成）。

---

## 任务生命周期：你的命令流

```bash
# 1. 创建（边界从最小开始，可参考下方自动推断表）
ctl task create --id <id> --objective "<text>" \
  --read-scope <path>... --write-allow <path>... --gates <gate>...

# 2. 推进
ctl task ready  --id <id>     # 人工放行用 approve
ctl task start  --id <id>     # → InProgress，写入边界生效

# 3. 在 write_allow 内实现；越界 → revise 加宽（需批准）或 ctl apply 申请例外
#    red→green 用 --tdd 互锁证明（见下）

# 4. 验收
ctl gate run --id <id> --gate <gate>     # 每个需要的 gate，记录 evidence

# 5. 提交审计
ctl task submit --id <id>     # → Review，提交窗口在此打开

# 6. 完成审计（read-only reviewer，reviewer ≠ implementer）
CTL_ACTOR=<reviewer-id> ctl review accept --id <id> --note "<summary>"   # 通过
CTL_ACTOR=<reviewer-id> ctl review reject  --id <id> --note "<findings>" # 驳回 → 返工

# 7. 完成 + 归档
ctl task finish  --id <id>     # 硬闸门（见下）
ctl task archive --id <id>
```

**规范顺序：submit →（在 Review 阶段：记录通过审计 + 提交边界内改动，两者都在 Review）→ finish → archive。** `ctl task finish` 是硬闸门，缺以下任一条都拒绝：

1. 最近一次 `submit` 之后记录的、新鲜的 `ctl review accept`（**自己不能给自己记 passing 审计**；上一轮 pass 在重新 submit 后失效）；
2. 绑定到当前代码树的新鲜 gate evidence；
3. 干净的工作树。

finish 报「stale evidence」时：重跑 gate（`ctl gate run`）→ 重新审计（`ctl review accept`）→ 再 finish。

### 边界自动推断（从这里开始，然后最小化）

| 信号 | write_allow |
|---|---|
| 单文件修复 | 仅那个文件 |
| 模块改动 | 那一个模块目录 |
| 跨模块 refactor | 每个模块一条 |
| schema 改动 | schema 目录 + 所属 domain 模块 |
| 加测试 | 测试目录 或 对应源码目录 |

`write_allow` **永远最小**；要加宽只能用 `ctl task revise`（仅 Planning 阶段）或经批准的 `ctl apply`。

---

## 工作流技能（phase map）

skills 管「在什么阶段想什么」；每个 skill 声明自己的 station 契约（上游 artifact → 产出 → 下游消费者）。阶段按序运行，前置条件已满足的可跳过（来自 [workflow-skills.md](./.agent/protocols/workflow-skills.md)）：

| 阶段 | skill / 特性 | 何时触发 | 产出 |
|---|---|---|---|
| 1. grill / 第一性原理 | `ctl-grill-with-spec` | PRD 或实现之前，需求模糊/过宽/高风险/可能做错时 | 对齐 artifact（观察事实、规则、假设、不可约约束、目标/非目标、未知、最小实验）——**不是真理声明** |
| 2. PRD | `ctl-to-prd` | 上下文够了、要生成多个持久任务之前 | PRD，区分 **ObservedBasis**（你实际读到的）/ **ConfirmedBasis**（用户或项目权威确认的）/ **OpenUncertainty**（未决未知，绝不隐藏）；状态 draft·confirmed·superseded |
| 3. tasks | `ctl-to-tasks` | 确认的 PRD/计划 → 任务 | 垂直切片，每个独立可验证，声明 objective/scope/gates/acceptance/**AFK/HITL** 标签/blocking uncertainties |
| 4. TDD | `--tdd` 互锁（**特性，非 skill**） | 实现期 | 一次一个行为，red 证据先于 green 证据，green 后才 refactor |
| 5. diagnose / Bayesian | `ctl-diagnose` | bug、flaky、意外结果、架构不确定 | 可证伪假设排序 + 证据分级；**没有 red-capable 反馈回路前不提 fix** |
| 6. 架构评审 | 思考指南（`.ctl/spec/guides/`） | — | read-only，产出候选方案，不产出代码改动；用户选哪个候选变成新的受治任务 |
| 7. handoff | `ctl handoff`（**特性，非 skill**） | 会话变长/上下文变高/切换 agent 或平台/AFK 前 | 可移植的任务快照 |
| 8. decision map | 思考指南 | grill 发现 fog（要等前沿推进才能定的决策）时 | 项目级索引（Destination·Frontier·Fog·Out of scope） |

**配套 skills**：

- `control-guard`——控制平面入口，主动路由任务生命周期（scope/gates/audit/finish），宿主 hook 强制边界并注入 context。发现物遵循 **Iron Law：Symptom → Source → Consequence → Remedy**。
- `ctl-review`——read-only 评审子 agent。两种模式：(A) 变更前 edit review；(B) `submit` 后的 completion audit（跑闭环清单，要 build/test/lint **evidence** 而非断言）。这个 gate 是硬的。
- `ctl-cognitive`——编排认知+知识层（brainstorm / uncertainty / research / handoff / prd / ralph）。
- `ctl-spec`——`.ctl/spec/` 生命周期：首次 bootstrap，之后把设计决策/模式/坑/根因写进去。

### 每个阶段都遵守的不变量

- 产出 **artifact，不是 claim**；
- **draft 与 confirmed basis 分开**；披露 open uncertainty 而非隐藏；
- **red before green**：同一行为没有先前的 red 证据，就不能有 green 声明；
- **没有复现回路就不提 fix**；
- **架构评审是 read-only**；refactor 要开新的受治任务；
- 外部工作流灵感是 **L0 参考材料**（见 Provenance）——绝不是权威，绝不作为 active control 被 vendor。

> **框架是 placed，不是 floating**：第一性原理放在 grill / 设计澄清；Bayesian 推理放在 diagnose / 打破循环。不要造泛泛的「想得更好」skill。

---

## 边界与 hook：你必须知道的

- **写工具（Write/Edit/MultiEdit）是路径作用域、fail-closed 的。** `ctl` 不可用时它们**拦截而非放行**。范围内修改**优先用写工具**。
- **Bash 不是硬写边界。** Claude Code 的 Bash 在 `ctl` 出错/超时时 **fail-open**（绝不锁死 shell），且**不做路径作用域检查**——所以 bash 不能用来可靠地约束写入；越界的 bash 文件变更（静态可识别目标的）会被**拒绝**而非 observe。需要 bash 改文件时，优先改用 Write/Edit。
- **受保护路径永远硬拒**：`.git/`、`.ctl/tasks/*/events.jsonl`、`schemas/`、`Cargo.toml`、`Cargo.lock` 等。**绝不手改 `events.jsonl` 或 `task.json`。绝不绕过 gate。**
- **子 agent 派发（平台边界）：**
  - **只派发只读研究/探索类子 agent**（始终安全，无写入无绑定问题）。
  - **写操作留在主 agent。** ctl 通过 `CTL_TASK_ID` 把一次写入绑到任务；**子 agent 不继承环境变量**，派发出去的写会丢失任务绑定（只在恰好一个 active task 时碰巧能用）。Claude Code 的 PreToolUse **不门禁 `Task`/子 agent 生成工具本身**。详见 [.claude/subagent-dispatch.md](./.claude/subagent-dispatch.md)。
  - OMP / opencode 的 `task` 派发经会话级插件门禁，opencode 的 Bash 亦 fail-closed。
- **多 active task 时显式 bind。** 一个被门禁的工具调用由派发它的任务治理。多于一个 active task 时，**显式绑定**目标 task id（宿主转发给 `ctl hook gate`）。**不要**依赖「只有一个 active task」的隐式回退——那是附带行为，不是契约。

---

## 认识状态层（record-and-disclose）

> **这是记录与披露，不是验证。** ctl 不证明思考发生过，也不证明结论正确；它只如实记录哪些运行/产物发生过、来自谁/什么、是否经过独立挑战，以及哪些未知被何种证据关闭。**产物存在 ≠ 思考质量存在；独立调用存在 ≠ 正确性存在。** 详见 [EPISTEMIC_CONTROL.md](./EPISTEMIC_CONTROL.md)。

- **Brainstorm 来源**（`ctl brainstorm record|attach-critic|skip-critic|show`）：把一次思考的发散/挑战/收敛产物按 path+hash 绑到任务。**记录-only**——从不门禁 create/finish，从不声称「思考有质量」。产物本身（L0）放在受 git 跟踪的 `brainstorms/<id>/`，不进 `.ctl/`。
- **不确定性账本**（`ctl uncertainty record|evidence|dispose|status`）：把任务携带的未知显式记下，以 `resolved`/`accepted_as_assumption`/`invalidated` 了结。`resolved` 必须引用一条 oracle-typed evidence，其来源被如实披露。**`model` oracle 是顾问性的，不能 resolve 一个未知**——控制层在命令层拒绝以 model 证据 resolve。
- **研究 / Spike**（`ctl task create --kind research` + `ctl research record|status`）：以证据 + 不确定性了结为产出，而非代码。
- **子 agent 派发归属**（`ctl dispatch record|list`）：在 parent 任务账本上记录派发。host-attested——ctl 记录「被告知派发了什么」，**从不验证实际跑了什么**。

---

## HITL 节点：你必须停下等人的地方

在这些点上停下，把决定权交给人：

1. **确认边界**——`ctl task create` 后、`start` 前，把推断的 objective/scope/gates 给用户确认（`yes / 调整 / skip`）。grill 的产出在用户确认前不构建。
2. **approve（人工 ready）**——需要人来放行的任务，用 `ctl task approve` 而非 `ready`。
3. **完成审计的 verdict 记录**——`ctl review accept/reject` 的记录者必须 ≠ 实现者（用 `CTL_ACTOR` 区分）。这是角色标签，不是被证明的独立身份。
4. **确认归档**——`ctl task archive` 前让人确认。

> **诚实披露：** telemetry、model/oracle 输出、人工回填都是 **evidence，不是 state**——它们从不放松 scope，一个未知信号 fail closed。把任务携带的未知记下来并披露；**绝不**把 model 判断当成 verdict。

---

## red→green TDD（--tdd）

用 `--tdd` 把一个任务选入 red→green：tdd-red-green 互锁会在账本上证明它（一次一个行为；red 证据先于 green 证据；green 后才 refactor；测公共行为而非私有实现细节）。它是**特性不是 skill**——它不证明正确性，只是在账本上把纪律落到实处。

---

## 交接与上下文压缩

会话变长、上下文变高、切换 agent/平台、或 AFK / 起一个独立受治 run 之前，压缩上下文：

```bash
ctl handoff export --id <id>                 # 导出只读任务快照（--json 拿机器可读）
ctl handoff capture --id <id> --file <json>  # 持久化 agent/人工判断（决定/未知/风险/下一步安全动作）
```

它**只读、不发事件**——是给下一个 session 或人来接手的可移植快照。

---

## 一个完整的 agent 驱动示例

```text
[triage]   用户：「登录态老过期，帮我修。」→ 多文件、有目标 → 创建任务
[align]    /ctl-grill-with-spec：第一性原理 + 一次一问面谈
           产出对齐 artifact；报告 station；等用户确认（HITL #1）
[create]   ctl task create --id 07-05-fix-session \
             --read-scope src --write-allow src/auth --gates cargo_test
           → 用户确认边界（HITL #1）
[ready]    ctl task ready --id 07-05-fix-session
[start]    ctl task start --id 07-05-fix-session
[implement]在 src/auth 内实现（越界改 src/config.rs → ctl apply 申请例外）
           选了 --tdd：先写失败测试（red）→ 实现 → 转 green
[gate]     ctl gate run --id 07-05-fix-session --gate cargo_test
[submit]   ctl task submit --id 07-05-fix-session   → Review
[audit]    派 read-only ctl-review 子 agent 跑 completion audit：
             build/test/lint evidence + 回看 ctl decisions 的 observe 记录
           CTL_ACTOR=ctl-review ctl review accept --id 07-05-fix-session --note "..."
                                                            （HITL #3，reviewer≠实现者）
[finish]   ctl task finish --id 07-05-fix-session    （硬闸门通过）
[archive]  ctl task archive --id 07-05-fix-session   （HITL #4，等人确认归档）
[spec]     /ctl-spec：把这次的设计决定/坑写进 .ctl/spec/
```

---

## 反模式（禁止）

- **多文件开工却不创建 ctl 任务。**
- **写到 `write_allow` 之外**，或把 scope 扩到根。
- **手改 `events.jsonl` 或 `task.json`。**
- **给自己记 passing 完成审计**（reviewer 必须 ≠ implementer）。
- **没诊断根因就提 fix**（Iron Law：先 Symptom→Source→Consequence，再 Remedy）。
- **把 model/evidence 输出当成权威 state**（它是 evidence，从不放松 scope）。
- **靠「只有一个 active task」的隐式回退**绑定任务。
- **把 writable 写操作派发给子 agent**（写留主 agent）。
- **跳阶段**（如 Planning 直接到 InProgress 而不经 Ready）。

---

## 去哪找更多

| 想了解 | 看 |
|---|---|
| 每条命令的精确语义 | [USAGE.md](./USAGE.md) + `ctl --help` |
| 项目身份、架构、禁项 | [AGENTS.md](./AGENTS.md) |
| 控制论闭环、drift、schema | [DESIGN.md](./DESIGN.md) |
| 认识状态层、四级信任 | [EPISTEMIC_CONTROL.md](./EPISTEMIC_CONTROL.md) |
| 治理协议原文 | [`.agent/protocols/control-guard.md`](./.agent/protocols/control-guard.md)、[`.agent/protocols/workflow-skills.md`](./.agent/protocols/workflow-skills.md) |
| 子 agent 派发的研究结论 | [`.claude/subagent-dispatch.md`](./.claude/subagent-dispatch.md) |
