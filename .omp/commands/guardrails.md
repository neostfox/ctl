根据 `ARCHITECTURE_GUARDRAILS.md` 审计当前请求、已声明的实施合同和工作区变更。

使用 `control-plane-guardrails` skill 工作流。

面向用户的报告默认使用中文。保留 `ALLOW / ASK / STOP`、`audit_hold`、文件路径、命令和机器字段的原始形式。

返回：

```text
当前里程碑
当前运行模式和已声明的实现 scope
ALLOW / ASK / STOP 判定
受影响的围栏 ID
预期能力和文件
实际修改的文件
scope 偏差
依赖变更
schema 变更
顶层模块变更
新增副作用
验证命令和结果
协议覆盖缺口
未命中 STOP 时的漂移分数
建议的下一步
```

审计期间不要编辑文件。

如果运行模式为 `audit_hold`，报告判定后继续保持只读。等待用户运行 `/guardrails-resume` 或 `/guardrails-explore`。不要建议用户手动执行被项目围栏拦截的命令。

如果 Git 元数据不可用，明确说明该限制，并审计实施批次报告的文件变更列表。
