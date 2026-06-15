# Epistemic Control — ctl 的认识状态控制边界

> 状态:设计冻结(design-frozen),尚未实现。本文件定义 `ctl` 在"执行控制"之上要补的一层——**认识状态控制(epistemic state control)**——的问题、边界、原则与最小本体。它描述目标状态,不描述当前已实现的能力。

本文件是五轮设计评审的收敛产物。它的职责是把一组分散的评审意见,固化成一条 `ctl` 必须长期遵守的产品边界,防止后续开发在错误的抽象上扩张(尤其防止把 `control plane / audit / sandbox` 等词理解为当前实现能提供的安全保证)。

---

## 1. 问题定义

`ctl` 已经较好地闭合了**执行控制环**(scope → implement → gate → review → complete),但存在五个语义断点。前三个是可直接验证的**一致性**问题,第四个是需要外部挑战的**认识**问题,第五个是一条根本的**可观测性边界**。

1. **代码不绑定被验证的代码** — `gate_checked` / review / completion 证据不携带 `tree_hash`;`finish` 的"新鲜度"按事件 `seq` 判定,而非按代码身份。结果:在 commit A 上通过的 gate/审计,可被随后提交的 commit B 静默继承(rebase / fmt / 补丁 / 多 Agent 交叉提交都会自然触发)。**这是在单人、诚实、无对手的情况下也会发生的正确性缺陷。**
2. **身份不绑定可信主体** — actor 来自 `CTL_ACTOR` 环境变量(缺省 `"human"`),no-self-approval 是 actor 字符串比较。所谓"职责分离"是诚实调用者的约定,不是密码学/OS 意义上的 separation of duties。
3. **实现不绑定需求/设计** — `TaskState` 无一等公民的需求/设计引用;唯一名为 `acceptance` 的字段实际编码的是"所有 gate 通过 + scope 强制",即**过程合规**。领域概念发生折叠:Process Acceptance 被当成了 Requirement Acceptance。
4. **需求和设计未被独立挑战** — 当使用者只提供方向、由 AI 推演需求与设计时,意图不再活在任何人脑中。一个全绿、全绑定、全一致的任务仍可能建在 AI 幻觉出的 spec 上,而使用者无力发现。
5. **认知产物缺少可验证的来源、独立性与挑战记录** — brainstorm / 推理过程是提示词路由约定,不是控制层不变量。没有事件或产物记录它是否真的发散过、是否有独立 critic、产物来自谁。

> 第五条的措辞刻意是"认知产物缺少可验证的来源、独立性与挑战记录",**不是**"认知过程不绑定是否发生"。后者暗示控制器能验证思考;前者只要求控制器验证它真正能观察到的事实。

---

## 2. 根本边界

- **思考过程不可观测。** 能被记录的只有:运行(run)、产物(artifact)、来源(provenance)、挑战痕迹(challenge)和外部证据(evidence)。
- **`canonical envelope integrity ≠ enclosed claim trustworthiness`。** 一个引用存放在哪个账本层级,不决定它内部声明的可信等级。把一个自报的 `critic_run_id` 塞进一条 canonical event,事件完整性是真的,但"独立性"那句话仍然是自证的。信封层级与内容可信是**两条独立的轴**。
- **谁来 attest attestor,是独立编排器 / 信任根问题。** 把信任从"skill 自报"挪到"executor 记录",只有当 executor 是一个**实现 Agent 无法假冒的独立信任域**时才是真进步。在当前架构里(`ctl run start` 由 Agent 经 CLI 调起,run 字段不含 `model_id`/`context_hash`),这个独立编排器不存在,因此 run provenance 仍是"结构更好的自证"。这与"身份不绑主体"是同一个信任根问题。

> 因此第五条不是普通 bug,也无法被"彻底修复"。它是一条产品必须长期遵守的边界:**不要把产物存在,当作思考质量存在;不要把独立调用存在,当作正确性存在。**

---

## 3. 产品原则

> **ctl 不证明思考发生,也不证明结论正确;它只证明哪些可观察的运行和制品发生过、由谁或什么产生、是否经过独立挑战,以及哪些未知被何种证据关闭。**

推论(不确定性披露自身也必须带来源):

> ctl 还必须暴露**那张不确定性地图本身的来源**——是谁/什么画的、有没有独立的东西校验过它。否则一份自报的、结构精美的未知清单,只是一个穿了实验室白大褂的绿勾。

---

## 4. 四级信任

| 级别 | 含义 | 示例 | 性质 |
| --- | --- | --- | --- |
| **L0** | 自由文本 / Skill 裸产物 | `brainstorms/<id>/`(divergence/critic/convergence) | 可直接修改、无 seq、不经 reducer。**不得称为审计证据**,只是"未验证的内容制品"。 |
| **L1** | fact index / 事实索引 | `telemetry.jsonl` | 可查询、可关联,仍可被本地修改。 |
| **L2** | canonical event / 参与 replay | `gate_checked`、`brainstorm_artifact_recorded` | 经应用层验证并追加,有 seq、幂等、可重放。 |
| **L3** | 防篡改证据 | 单写者 / hash chain / 签名 / 外部锚定 | 抗对手。当前未实现。 |

> **关键:信封层级(L0–L3)和内容可信等级是两条独立轴。** 一条 L2 事件可以包裹一个 L0 等级的声明(例如自报的 `critic_run_id`)。事件证明"它被如实记录了",不证明"它描述的事实为真"。文档和 UI 必须按内容的真实等级标注,不能因为文件位于 `brainstorms/` 下、或因为它进了 canonical event,就把它渲染成控制层事实。

> **目录边界(与强制实现一致):** `.ctl/` 存放**受保护的控制状态**(`events.jsonl`、`task.json`/`control.json` 投影、配置)——`PathNormalizer` 拒绝对整棵 `.ctl/` 树的写入,Agent 不得直接写。**认知产物(L0)放在受 git 跟踪的顶层 `brainstorms/<id>/`**,而非 `.ctl/`:L0 是可直接修改、未验证的内容制品,跟踪它只是为了让 path+hash provenance 可持久,绝不因此提升其可信等级。

---

## 5. 最小本体

刻意收敛,避免重蹈"DESIGN.md 设想 PRD/coverage,代码啥也没有"的愿景>实现裂缝。

### 5.1 单一 `Uncertainty` 生命周期

不建设 Claim / Evidence / Unknown 三套重本体——一个"有证据的 Claim"本质就是一个"被证据关闭的 Unknown"。只保留一个对象:

```
open → resolved(evidence_ref) | accepted_as_assumption | invalidated
```

最小字段:`id`、`statement`、`source_run_id`、`resolution`(可空)。`impact` 四级、`blocking`、`confidence`、各种关联,都留作后续 schema 演进,**第一版不做**。

- `resolved` **必须** 带 `evidence_ref`,且要可区分"被断言关闭"与"被外部 oracle 关闭"。无外部证据就 resolved 的未知,只比 open 好一点点。

### 5.2 两种 Binding(+ 一个查询)

```
ObservedBasis    实际参考过的版本(开始实现时即记录,只作追溯,不作门禁)
ConfirmedBasis   被明确重新确认的版本(提交/评审/完成时的权威绑定)
```

`CompletionBasis` **不是第三种角色**,它是一个查询——"finish 时刻那个 ConfirmedBasis 是哪个"。**派生量不持久化。**

原则:**从头记录 hash,选择性 gating。** 早期 hash 只作 provenance,spec 稳定后才把某个 hash 升格为完成不变量。统一抽象是 `BoundRef`(`tree_hash` / `policy_hash` / 可选 `requirement_hash` / 可选 `design_hash`),两条链(执行链、需求链)共用同一个原语。

---

## 6. 披露模型

完成时**禁止单一绿勾**。状态压缩(把过程、制品、认识、治理压成一个 `PASSED`)是本设计要消灭的核心危险。四维分开披露:

| 维度 | 是否允许 verdict | 说明 |
| --- | --- | --- |
| **Process** | ✅ 允许 | 对规则的二元判定(scope/gate/流程)。 |
| **Artifact** | ✅ 允许 | hash 比较(CURRENT / STALE)。 |
| **Governance** | ✅ 允许(事实性) | 独立主体有/无;不打分。 |
| **Epistemic** | ❌ 不允许 | **只披露原始纹理**:open assumptions、blocking unknowns、high-impact risks、unknowns closed by external evidence、independent critic 有无。**不给总评、不给分数、不给红黄绿灯。** |

> Epistemic 一旦被赋予一个词(如 `PARTIALLY_VALIDATED`),就又造了一个更小的绿勾——它要么变成所有人学会无视的黄灯,要么被反向操纵(标 resolved 升级标签)。独立性同理:`separate invocation` / `separate context root` / `different model` / `blind first pass` 都是可披露的**事实**,但不得汇总成 `independence score: 82%` 或 `sufficiently independent`——"同模型不同上下文"与"不同模型同训练分布"之间不存在可靠的通用换算。

---

## 7. Brainstorm 最小落地

- **保留单一 `ctl-brainstorm` skill**,但内部要求产出两份独立制品:`divergence artifact` 与 `convergence proposal`,critic 插在二者之间:

  ```
  模糊请求 → originator divergence → independent critic → revised uncertainty set → convergence / Task Proposal
  ```

- **不记录 `brainstorm_completed`。** `COMPLETED` 会被读成"已充分思考"。只披露事实状态(originator artifact 有无、critic 独立运行的各维度事实、critic-raised / externally-resolved / open-blocking 未知计数;或 `independent challenge: none`;或 `skipped + reason + decided_by`;或 `provenance: unrecorded`)。
- **出口条件不是"skill 被调用过",而是 Task Proposal 携带 provenance:** `originator artifact ref` + (`critic artifact ref` 或 `skip disposition`)。**不 gate "是否真正思考过",gate 产物的 provenance。** 强制点放在 `ctl task create` 出口,而非 brainstorm 入口(入口靠自评歧义度会构成控制回路反转)。
- **`critic_runs` 不是元数据,是证据本身**,且必须由真正发起 Agent 调用的**运行层**记录(`originator_run_id` / `critic_run_id` / `model_id` / `context_root_hash` / `instruction_hash` / `output_artifact_hash`),不能由 brainstorm skill 自己写成事实——否则它和 `actor: "human"` 没本质区别。
- **最小可观测单元是 originator 与 critic 之间的 delta**,不是 `brainstorm.json` 本身。一份 `critic_runs: []` 的 brainstorm 是 originator 的自画像,必须如实披露为"未经挑战"。
- **`skip_reason` 必须绑定决策来源:**

  ```
  SkipDisposition { reason, decided_by: RunRef, recorded_at, evidence_ref? }
  ```

  枚举:`trivial_change` / `unambiguous_existing_spec` / `pure_read_only_analysis` / `emergency_fix` / `explicit_user_skip`。`decided_by` 必须让人看见:是用户跳过的,还是被监督的 Agent 自己决定不用监督(与"风险等级不能由实现者自报"是同一控制原则)。

---

## 8. V1 Canonical Events

第一版只引入三种事件(不必存全部内容,只存引用与 hash):

- `brainstorm_artifact_recorded` — `{ brainstorm_id, artifact_hash, produced_by_run }`
- `critic_artifact_attached` — 必须引用由执行器生成的独立 `run_id`,不接受 JSON 内自报
- `brainstorm_skipped` — `{ reason, decided_by, evidence_ref? }`

> **明确声明:在独立编排器出现之前,`critic_run_id` 只能是"未认证声明"(unattested claim),不得宣传为已证明的认识独立性。** 把它装进 L2 事件,提升的是信封完整性,不是内容可信。

---

## 9. 明确不做(Non-goals)

- `Spec confidence: 87%` 这类伪精确分数(它会成为另一种绿勾)
- PRD 管理子系统
- 需求覆盖矩阵(requirement coverage graph)
- 主动失效传播引擎(失效用惰性比较 `current_digest != recorded_digest` 派生,不持久化、不主动遍历下游)
- 硬阶段门(forward-integrity 顺序门;改用 hash 绑定 + 失效 + 显式 rebind,顺应设计涌现)
- 对 brainstorm 深度或正确性的判断
- 将自报字段经 canonical event 包装后冒充可信事实

---

## 附:修复优先级(供路线图参考,非本文件强制)

1. **立即(执行结论正确性):** `tree_hash` 绑定 → `policy_hash` 绑定 → gate/review/audit/finish 制品一致性惰性校验 → 事务化单写者 → Gate 超时终止进程树。
2. **企业治理前:** authenticated principal → approval 签名 → actor/role 授权 → hash chain。(principal 是否先于单写者,取决于近期是否真要支持多人审批。)
3. **多 Agent 前:** 可选 `requirement_hash`/`design_hash` 惰性绑定 → 独立 design-critic(依赖 authenticated principal 才有真意义)→ 显式 rebind。
4. **认识状态可见性(对"模糊想法"工作流是产品价值 P0,与执行链同期):** 风险驱动的 externalize-before-implement → `Uncertainty` 最小结构 → finish 分维度披露 → research/spike 任务类型 → 开始实现时记录 `ObservedBasis`。

---

*本文件的定位:`ctl` 是 AI 研发过程中的**可观察事实、证据来源与剩余不确定性控制层**——它绑定结论的来源、暴露剩余的不确定、并暴露这份不确定性记录自身的来源与独立性。它不制造正确性,也不评判正确性。*
