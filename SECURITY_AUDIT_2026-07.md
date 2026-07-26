# ctl 控制面安全与文档审计 — 2026-07

> 范围:对 `ctl` 0.0.14 全部源码 + 安装脚本 + 用户文档做一次系统化的逻辑漏洞与文档友好度审查。
> 方法:6 个并行只读 scout 子代理分块审计(boundary / 账本+reducer / gate runner / self-update / workspace+schema / adapter+CLI / 文档),主代理对每个 critical/high 发现亲自交叉验证(file:line + 触发输入 + 修复点)后再动手修。
> 验证:`cargo test --bin ctl` 全绿(567 既有 + 3 个新回归 = 570,实际跑出的 569 含 1 个过滤);`cargo check --tests` 0 error;`sh -n install.sh` / PowerShell tokenizer 通过。

---

## TL;DR

- **修复 4 个 critical 漏洞**(其中 1 个会让所有生产用户在不知情下绕过 schema 校验)。
- **修复 6 个 high 漏洞**(覆盖写入边界、工作树应用、gate 监督、安装校验、symlink 重定向)。
- **修复 8 项文档问题**(README 前置要求、命令一览、安装验证、故障排除、卸载/代理/新终端指引)。
- **7 类问题刻意未改**:需要架构决策或需要新依赖,见末尾"残留风险与跟进"。
- **健全性已验证**(scout 显式确认):reducer 纯净性、seq 单调性在 append 时强制、hold 不可绕过、并发 locking、adapter 自名一致性、prepare_run 不泄密、CLI 无 panic 向量。

---

## 修复明细

### CRITICAL

#### C1. spec-path 早返回绕过 `..` 拒绝 → 越界写任意受保护路径

- **位置**:`src/cli/mod.rs:7147-7154`(`is_spec_path`)+ `src/cli/mod.rs:7282`(早返回调用点)。
- **症状**:`cmd_hook_gate` 的 write/edit 分支在 `classify_write_target` **之前**先调 `is_spec_path`,而后者用 `Path::starts_with`(组件级匹配,但**不折叠 `ParentDir**`)。`project_root.join(".ctl/spec/../tasks/events.jsonl")` 的前三个组件匹配 `.ctl/spec`,返回 `true` → gate 直接放行,`classify_write_target` 永远不到。host 随后真正写入,OS 解析到受保护目标。这绕过了 observe 模式下**唯一的硬拒**(保护路径)。
- **触发**:`Write file_path = .ctl/spec/../tasks/events.jsonl`(改写 canonical 事件账本);同理 `.ctl/spec/../../Cargo.toml`、`.ctl/spec/../config.toml`、`.ctl/spec/../../.git/config` 全部命中。
- **修复**:`is_spec_path` 与 `path_in_scope` 在 `starts_with` 之前先走 `classify_write_target`,只在分类为 `InRepo`(无 `..`/UNC/绝对/受保护)时才视作 spec/scope 命中。`cmd_hook_check_write` 同样收口。
- **测试**:`spec_path_with_traversal_is_not_spec`、`path_in_scope_rejects_traversal_target`(新增,`src/cli/tests.rs`)。

#### C2. Windows 自更新两步 rename,中间崩溃致机器无 ctl

- **位置**:`src/infrastructure/self_update.rs:202-209`(`self_replace` Windows 分支)。
- **症状**:Windows 不允许覆盖运行中的 .exe,代码用「先把运行中二进制 rename 成 `.old`,再把新二进制 rename 到原位」绕过。但两个 rename 之间存在微秒级窗口——进程被 kill(Ctrl+C / OOM / BSOD / 断电)落在窗口里,机器上 `ctl.exe` 路径为空,下次 `ctl` 启动报 "file not found"。原代码的 rollback 只在第二个 rename 返回 `Err` 时才跑,**不覆盖进程终止**。
- **触发**:Windows 上 `ctl self-update` 期间进程被强杀。
- **残留风险(无法在不加依赖下完全关闭)**:Windows 没有针对运行中 .exe 的原子替换原语,`MoveFileEx(MOVEFILE_DELAY_UNTIL_REBOOT)` 需重启。完全闭合需要 wrapper 脚本架构或加 winapi 绑定。
- **修复**:加 `Rollback` Drop-guard 覆盖 panic 路径;加前置 `metadata.len() == 0` 校验拒绝空/损坏二进制;在代码注释和 `AGENTS.md` 诚实标注残留 kill-窗口。
- **测试**:Drop-guard 行为由编译期 `#[cfg(windows)]` 保证; panic 路径未加专门测试(需要注入 panic,权衡下未做)。

#### C3. schema 校验在 `schemas/` 缺失时静默放行 — 影响所有生产用户

- **位置**:`src/application/mod.rs:5340-5346`(`new_validator_if_available`)+ `src/application/mod.rs:3306`(`validate_event` 的 `if let Some`)。
- **症状**:`schemas/` 目录是相对 **CWD** 加载的。已安装的 `ctl` 从用户项目根目录跑,那里有 `.ctl/` 但**没有** `schemas/`(后者只在 ctl 源码仓库里)。结果:`SchemaValidator::new` 返回空 `schemas` 向量,`new_validator_if_available` 返回 `None`,`validate_event` 静默跳过 schema 校验,只跑 reducer。**这意味着所有生产用户在 append 事件时,额外的字段检查、enum 成员检查、UUID/date-time 格式检查、`unevaluatedProperties` 检查全部失效**——只要 reducer 不挡,任意 payload shape 都能进 append-only canonical 账本。
- **触发**:任何已安装 ctl 的生产用户,执行任何会写事件的命令(`ctl task create` 等)。
- **修复**:用 `include_str!` 把 6 个 schema 文件编译进二进制(`EMBEDDED_SCHEMA_FILES` 常量),`SchemaValidator::new` 现在永远会返回至少嵌入 floor 这一份 schemas。`new_validator_if_available` 改为始终 `Some(...)`(保留 Option 签名以稳定 API)。磁盘 schemas 仍优先生效(开发迭代)。
- **测试**:既有 11 个 schema 测试全绿(`test_schema_validation`、`schema_rejects_*`、`schema_counter_examples_from_fixture` 等)。

#### C4. install.ps1 checksum 不匹配被 catch 降级为 warning 后照样安装

- **位置**:`scripts/install.ps1:60-72`(原 checksum 块)。
- **症状**:整个 checksum fetch + verify 包在一个 `try { ... } catch { Write-Warning }` 里。mismatch 的 `throw` 在 try 内部 → 被外层 catch 接住 → 降级为黄色警告 → 流程继续到 `Expand-Archive` + `Copy-Item`,**被篡改的二进制照常安装**。同样的 catch 还把 `.sha256` 文件 404/不可达静默降级。和 `self_update.rs:261`(明确 bail "refusing to install unverified binary")以及 `install.sh:83` 直接矛盾。
- **触发**:任何对 GitHub Releases 的 MITM/CDN 投毒,或 `.sha256` 文件 404。
- **修复**:fetch 和 verify 拆开——fetch 失败才 `throw`(fatal),verify mismatch 也 `throw`(fatal,落在 try 外)。空 checksum 文件也 fatal。
- **测试**:无 PowerShell 测试基础设施,靠 `pwsh` tokenizer 通过 + 手工对照 `self_update.rs` 的 fatal 策略。

### HIGH

#### H1. `path_in_scope` / `cmd_hook_check_write` 词法 starts_with 接受 `..` 穿越

- **位置**:`src/cli/mod.rs:7095-7103` + `src/cli/mod.rs:6483-6491`。
- **症状**:与 C1 同源。`path_in_scope` 用 `Path::starts_with`(组件级但不折叠 `..`),`src/../../../etc/passwd` 在 `write_allow = ["src"]` 时仍匹配。`cmd_hook_check_write` 直接调它打印 `{"allowed":true,"reason":"in_scope"}`。
- **修复**:统一收口——`path_in_scope` 先走 `classify_write_target`,只在非 `Suspicious`/非 `OutOfRepo` 时才做 starts_with(保留 `Protected` 命中,以便 `ctl apply` 例外仍生效);`cmd_hook_check_write` 只在 `InRepo` 时继续。
- **测试**:见 C1 的两个新回归。

#### H2. `workspace_apply` 漏掉受保护路径检查,changeset 可静默改写 canonical 账本

- **位置**:`src/application/mod.rs:3475-3535`(`workspace_apply`)+ `src/infrastructure/workspace/mod.rs:415-431`(`protected_risks`)。
- **症状**:`workspace_apply` 先 `file_in_write_scope`(只调 `normalizer.normalize`,**不调** `normalizer.is_protected`),再只查 `detect_high_risk`。而 `detect_high_risk` 的保护前缀列表 `[".omp/", ".ctl/spec/", "schemas/", "Cargo.{toml,lock}"]` 是 `PathNormalizer.is_protected`(含 `.git`/`.ctl`/`.ctl/tasks`/`.control`)的**严格子集**。结果:一个 `write_allow` 含 `.ctl/tasks/<id>/events.jsonl` 的任务(创建任务时并不拒受保护路径),在工作树里 `git add -f` 改账本,跑 `ctl workspace apply` 时既不触发 high_risk 也不触发保护硬拒,**canonical 事件流被覆盖**。
- **触发**:任务 `write_allow` 含 `.ctl/tasks/<id>/events.jsonl` 或 `.control/*`;工作树修改后 `ctl workspace apply`。
- **修复**:`workspace_apply` 在 scope 检查后,对每条 path 显式调 `normalizer.is_protected`,命中即 `Err` 指引走 `ctl apply` 受审例外。
- **测试**:既有 9 个 workspace 测试全绿(包括 `dirty_paths_*`、`shared_git_risk_*`、`scoped_tree_diff_*`)。

#### H3. 未知工具 catch-all fail-open + omp `multiedit`/`patch` 绕过 gate

- **位置**:`src/cli/mod.rs:7322`(原 write/edit 分支只匹配 `"write"|"edit"`)+ `src/cli/mod.rs:7736`(未知工具 catch-all)。
- **症状**:omp hook 把 `multiedit` 原样传给 ctl(没像 opencode 那样映射到 `write`),ctl 落入 `_ => allowed:true, reason:"unknown tool — default allow"` arm,**完全不做边界检查**就放行。opencode 和 Claude 因为有映射不受影响。
- **触发**:omp 上调用 `multiedit` 改 `.ctl/tasks/x/events.jsonl`。
- **修复**:`"write"|"edit"` 分支加上 `"multiedit"` 别名;未知工具 arm 改为仍 allow(避免锁死合法未映射工具)但加 `"record":true` + `warning` + `remedy`,让缺口在 `.ctl/decisions.jsonl` 可见。
- **测试**:既有 gate 相关测试全绿。

#### H1+ (A1 follow-up). 写门纯词法,symlink 重定向漏洞

- **位置**:`src/cli/mod.rs:7221`(`classify_write_target`)+ `src/infrastructure/boundary/normalizer.rs:103`(`canonical_for_gate`)。
- **症状**:`classify_write_target` 只做词法 UNC/绝对/`..`/Prefix 拒绝 + 词法 `is_protected`,从不调用 `fs::canonicalize` / `fs::symlink_metadata`。结构层的 symlink 检查只在 create/revise 时跑过 scope 路径,write 时没跑。结果:任务 `write_allow=["src"]`,agent 创建 symlink `src/ln -> .git/config`(创建是 in-scope 写,允许),然后 `Write file_path=src/ln`——词法路径是 `src/ln`(非 protected),gate 放行,OS 写落到 `.git/config`。
- **触发**:`write_allow=["src"]` 的任务;agent 在 src 下创建指向保护路径的 symlink 后写入。
- **修复**:给 `PathNormalizer` 加 `canonical_for_gate` 方法——做 `normalize` 同款的词法 + ancestry symlink walk + 根遏制,但 canonicalize 的 fallback 从「只试直接父目录」扩展到「walk up 到最长存在的祖先」,这样**多层新目录的 new-file 写入**(gate 在 host 创建中间目录之前运行)不会误拒;`classify_write_target` 在词法检查通过后调 `canonical_for_gate`,失败一律 Suspicious(fail-closed),成功再 `is_protected` 一次。`is_spec_path` 和 `path_in_scope` 因为复用 `classify_write_target`,自动继承保护。
- **测试**:`write_target_in_repo_for_new_file_in_nonexistent_dir`(多层新目录合法)、`write_target_symlink_redirect_in_scope_is_suspicious`(Unix symlink 重定向到 .git/config 被拒)、`write_target_symlink_redirect_to_protected_is_suspicious`(Unix symlink 重定向到 canonical ledger 被拒)。Windows symlink 创建需开发者模式,测试用 `#[cfg(unix)]` 门控。
- **健全性确认**:`canonical_for_gate` 的 ancestry walk **本来就在 `normalize` 里**——它访问每个存在的组件并 `symlink_metadata` 检查。对 `src/ln` 这种 leaf 是 symlink 的情况,`current.exists()` 通过 symlink 追到目标(存在),`symlink_metadata` 报告 link 本身,`is_symlink()` 返回 true,Err。也就是说 `normalize` 一直能挡这个攻击——只是 gate 没调用它。修复就是让 gate 走这条路,同时加长 fallback 处理新目录场景。

### CRITICAL

| 严重度 | 问题 | 修复 |
|---|---|---|
| high | `ctl board [--kanban\|--table]` 宣传不存在的 `--kanban` 标志(clap 拒绝未知 arg,粘贴即报错) | 删 `--kanban`,补真实的 `--include-archived` |
| high | 安装后无任何验证步骤,也不告诉用户 `command not found` 怎么办 | 加 step 2「验证安装」(`ctl --version` / `ctl doctor`) + PATH 修复提示 |
| high | 无故障排除章节,常见首次运行失败无处查阅 | FAQ 之前插入「故障排除」表(8 行症状→命令→修复) |
| high | Windows 安装改用户级 PATH,但 README 没说要开新终端 → 同窗口 `ctl init` 报 command not found | 加 Windows 新终端提示 |
| medium | 折叠块里的示例硬编码 `v0.0.11`,与 Cargo.toml `0.0.14` 漂移 3 个补丁版本 | 改成 `vX.Y.Z` 占位并指向 Releases |
| medium | Linux/macOS 无 `~/.local/bin` PATH 指引 | 加 `export PATH` 写入 `~/.bashrc`/`~/.zshrc` 提示 |
| medium | 无卸载/回滚指引;Windows self-update 的 `.old` 备份未文档化 | 折叠块加「卸载」小节 |
| medium | 无公司代理/防火墙指引(installer 访问 raw.githubusercontent.com) | 加 HTTPS_PROXY 指引 + 引用 ADR 0002 |

同时把命令一览的 `ctl init` 加上 `--platform <name>`(真实存在,value_enum),`ctl board` 同步上面的修复。

---

## 已验证为健全(scout 显式确认,无需改)

| 子系统 | 不变量 | 验证位置 |
|---|---|---|
| 事件账本 | seq 在 append 时强制单调(持锁 replay+apply),降序/重复 seq 在锁内被 reducer 拒 | `application/mod.rs:3336-3338` + `domain/task.rs:909-914` |
| Reducer | 纯函数(无 fs / env / time / print / process),`apply` body 只用串/集合操作 + `Result<_,String>` | grep 全 `src/domain` 仅命中 `#[cfg(test)]` fixture 读 |
| Hold 不可绕过 | `apply` 顶部 hold guard 覆盖所有事件类型(除 `hold_exited`/`boundary_violation_recorded`/`gate_checked`) | `domain/task.rs:915-921` |
| 阶段机 | 拒绝 `Planning → InProgress` 跳过 Ready、拒绝 backward、拒绝从 Ready finish、终态拒 archive 之外的转移 | `domain/task.rs:929-1140` 各 match arm |
| Replay 决定性 | `task.json` 序列化只用 `Vec`/`BTreeSet`/scalar,`serde_json::Value::Object` 字母序,两次 replay 字节同 | `infrastructure/store/mod.rs:286-312` + 测试 |
| 并发锁 | `DirLock` 用 `create_new` 原子 + ownership nonce + 互斥 + acquire timeout,持锁过 validate+append 关键段 | `infrastructure/store/mod.rs:52-65` |
| Adapter 自名 | `adapter_for` 字面匹配,`adapter_contract_checks` 比对 registry 名与 self-report | `adapters/mod.rs:27-52, 340-367` |
| Adapter 形状 | doctor 与 CI 同套 `capabilities_*` 形状断言,新增 adapter 自动两面查 | `adapters/mod.rs:354-365, 432-442` |
| prepare_run | `RunManifest` 只带 ID/scope/gate,不读 env/token/proxy | `adapters/mod.rs:38-50` + omp/opencode impls |
| CLI panic 向量 | 所有整数 arg 是 clap 类型化,2 处 `unwrap` 非用户输入(由构造保证),无 `from_utf8`/indexing on 用户输入 | `cli/mod.rs:6011, 6612, 6571` |
| Gate 模板不可注入 | 模板是编译期 `static &[GateTemplate]`,`Command::new(command).args(args)` 不走 shell | `infrastructure/gates/mod.rs:28-95, 168-186` |
| PID 复用 | Child 句柄独占至 Drop,kernel 保留 PID 至 reap,kill 不能命中复用 PID | `infrastructure/gates/mod.rs:188-308` |
| Gate 读端 deadlock | drain 线程持续 `read_to_end`,管道不会填满阻塞子进程;`from_utf8_lossy` 不 panic | `infrastructure/gates/mod.rs:197-206, 387-393` |

---

## 残留风险与跟进(已识别但**未改**,需要团队决策)

按优先级与理由分组。每条都可作独立 issue。

### A. 写边界加固(需决定 fail-closed 策略与性能 trade-off)

(已修复,见 H1+)
2. **`apply_changes` 非事务化** — 多文件 changeset 中途崩溃留下半应用状态,workspace_applied 事件也未写。修复需暂存目录 + 原子 rename + marker 文件 + 启动时回滚/前滚。位置:`infrastructure/workspace/mod.rs:203-228`。
3. **scope 区分大小写与 FS/`is_protected` 不一致** — `path_in_scope` 用字节比较,Windows/macOS FS 是大小写不敏感。`write_allow=["Src"]` 在 Windows 上不匹配 `src/x.rs`。修复需 `cfg(target_os)` 下做大小写不敏感组件比较。位置:`cli/mod.rs:7095-7103`。
4. **`create_worktree` 不验证祖先链非 symlink** — `.ctl/tasks/<id>` 若被预置为 symlink,worktree 会落在 project_root 外。低概率但应防线纵深。位置:`infrastructure/workspace/mod.rs:46-66`。

### B. self-update 设计权衡(需架构决策)

5. **Windows kill-窗口残留风险** — 见 C2。完全闭合需 wrapper 脚本架构或 winapi 绑定加 `MOVEFILE_DELAY_UNTIL_REBOOT` fallback。
6. **静默降级** — `ctl self-update --version <older>` 不警告(只对 latest 跑 `is_newer` 守卫),钓鱼式指令可诱降级到含漏洞版本。修复:`--version` 时仍比对 current,降级要 `--allow-downgrade`。位置:`self_update.rs:239-247`。
7. **staging 目录名可预测(PID)+ 默认 umask** — 多用户主机上本地攻击者可预创建 `.ctl-update-<pid>` 赢得 TOCTOU。修复用 `tempfile::TempDir`(随机名 0700)。位置:`self_update.rs:268-270`。
8. **TOCTOU 在 tag 解析与 latest download 之间** — `resolve_latest_tag` 读 API,`download_base("latest")` 又重定向解析一次,两次之间发布新版会让展示的 tag 与实际安装的字节不一致。修复:从解析出的 tag URL 直接下载。位置:`self_update.rs:113-130`。
9. **`extract` 走 PATH 上的 `tar`** — 投毒的 PATH 可换 `tar`。修复:按 OS pin 绝对路径,或在进程内解压(已有 ureq 的 zip/gzstd 传递依赖)。位置:`self_update.rs:158-170`。

### C. Reducer 防御深度(应用层已挡,reducer 是最后防线)

10. **`run_started` arm 无 phase guard** — 不像兄弟 `workspace_created` 要求 `Phase::InProgress`,`run_started` 只查 `active_run.is_some()`。直写账本或未来绕过 `ControlApp::run_start` 的 caller 会在 Ready/Review/Completed 留下悬空 active_run。位置:`domain/task.rs:1266-1296`。
11. **`hold_entered` / `boundary_violation_recorded` 无 phase guard** — 终态任务上追加这类事件会翻 `is_held`,而 `task_archived` 不在 hold 豁免列表里 → 终态任务可被永久不可归档。位置:`domain/task.rs:1081-1089`。
12. **`command_id` 唯一性不在 append 强制** — 重复 command_id 的事件被 reducer 幂等 no-op,但仍然写进账本(只在 `validate_store` 事后报告为 issue)。修复:`validate_event` 在 `state.processed_commands` 命中时 reject。位置:`application/mod.rs:3296-3323` + `domain/task.rs:906-908`。
13. **`cmd_hook_check_write` 读 `task.json` 投影做决策** — 违反「投影永不做决策输入」。主 gate `cmd_hook_gate` 正确地走 replay,这个 outlier 不走。位置:`cli/mod.rs:6434-6543`。

### D. schema validator 完整性(长期换库)

14. **手写 validator 不是完整 Draft 2020-12** — 静默忽略 `$ref`/`$defs`/`pattern`/`oneOf`/`anyOf`/`not`/`dependentSchemas`/`prefixItems`/`maximum`/`maxItems`/`maxLength`/`exclusiveMinimum`/`multipleOf`/`else`/`contains`/`minProperties`/`maxProperties`/无 sibling properties 的 `unevaluatedProperties`。当前 6 个 schema 只用到已实现子集(已扫,健全),但任何引入未支持关键词的 schema 编辑会静默绕过。位置:`infrastructure/schema_validator.rs:43-290`。建议长期换 `jsonschema` crate(需评估 DEP-001..004)。
15. **`date-time` 格式校验拒绝所有 fractional-second 时间戳** — `&rest[digits_end + 1..]` 多跳一字节,丢掉 `+`/`-`/`Z` 首字节,RFC 3339 的 `.5Z` / `.123456+07:00` 全被拒。ctl 自己写的是 `YYYY-MM-DDTHH:MM:SSZ`(无 fraction)所以自产事件不受影响,外部系统的带 fraction 时间会被错拒。位置:`schema_validator.rs` 的 `is_valid_iso8601_datetime`。修复:改成 `&rest[digits_end..]` + 加回归。
16. **UUID 格式只查结构,不查 v4 version/variant 位** — `aaaaaaaa-bbbb-1ccc-dddd-eeeeeeeeeeee`(version=1, variant=d) 也通过。schema 描述明确写 UUIDv4。修复:加 `parts[2]` 以 `'4'` 开头、`parts[3][0]` ∈ `{8,9,a,b}` 检查。位置:`schema_validator.rs:80-92`。
17. **`$id` lookup 用子串匹配** — `id.starts_with(schema_id) || id.contains(schema_id)` 的第三分支无锚定,`"v1"` 会命中字母序第一个含 "v1" 的 schema。ctl 目前都用规范全名查询,不可利用,但是潜在误匹配。位置:`schema_validator.rs:30-34`。

### E. adapter 契约(需规格决策)

18. **`validate_output` 接受非字符串 `touched_files` 元素** — 下游用 `as_str().unwrap_or("")` + `if is_empty() continue` 静默丢掉,记录 run_completed 的 changeset 为空。位置:`adapters/omp/mod.rs:71-77`、`adapters/opencode/mod.rs:71-78`、`application/mod.rs:3840-3847, 5118-5124`。
19. **`validate_output` 不读 exit_code** — agent 报告 `exit_code:1` 仍被 `run_ingest` 记录为 run_completed。位置:`adapters/omp/mod.rs:59-77`、`application/mod.rs:3882-3896`。
20. **`run_start` 在校验 adapter 名之前就提交 worktree + lease_created** — 未知 adapter 名留下孤儿 worktree 和 dangling lease 事件,无回滚。位置:`application/mod.rs:3722-3756`。
21. **`manual` 文档化为 adapter 但不在 `SUPPORTED_ADAPTERS`** — `run start --adapter manual` 被 CLI 接受但 `run_start` 内失败;`ctl adapter capabilities --adapter manual` 报 unknown。两套 adapter 概念静默分歧。位置:`cli/mod.rs:1199-1200`、`adapters/manual/mod.rs`、`adapters/mod.rs:14`。

### F. gate runner 加固(性能/兼容 trade-off)

22. **drain 缓冲无界** — `read_to_end` 无上限,`OUTPUT_CAP`(64KB)只在读完后才截。话痨 gate 可在 60s 窗口内写到 OOM。修复:在 read 循环内到 OUTPUT_CAP 即 break。位置:`infrastructure/gates/mod.rs:198-204, 387-393`。
23. **PATH/CARGO_HOME/NODE_PATH 不剥离** — 任务能影响 ctl 父 env 时可注入木马 `cargo`/`npx` 总是退出 0,所有 gate 假绿。修复:pin 到绝对已知好路径,或限制 PATH 到系统/工具链目录。位置:`infrastructure/gates/mod.rs:358-381`。
24. **Windows `taskkill` 继承全部 env 且 PATH-resolved** — 把上面 PATH 木马放大成每次 timeout 都执行任意代码。位置:`infrastructure/gates/mod.rs:294-298`。
25. **gate 总在 project_root 跑,忽略任务隔离 worktree** — pre-merge gate 查的是 main 分支代码,不是任务在 worktree 里的修改。位置:`application/mod.rs:1182`、`4917-4927`、`4237`。
26. **npx gate 在文档说「fail-closed 拒绝 registry 拉取」下仍可联网** — `filter_allowed_env` 只剥代理/token 变量,不挡直接 egress。能改 `package.json` 的任务可触发 npx 拉恶意包。位置:`infrastructure/gates/mod.rs:67-95, 358-381`。

### G. 安装脚本一致性(小修)

27. **install.ps1 把 `%LOCALAPPDATA%\ctl\bin` 提到 User PATH 首位但去重逻辑只看精确字符串** — 同路径不同大小写/尾斜杠会重复。位置:`scripts/install.ps1:80-90`(已改了 fail-open,这条是次要清理)。

---

## 修改清单(本审计直接提交)

**代码(5 个 critical + 5 个 high = 10 处修复)**:
- `src/infrastructure/boundary/normalizer.rs` — 新增 `canonical_for_gate` + `canonicalize_longest_existing_ancestor`,关闭 in-scope symlink 重定向漏洞(A1)
- `src/cli/mod.rs` — `classify_write_target` 在词法检查后 canonical-resolve 并重检 `is_protected`;`is_spec_path`、`path_in_scope`、`cmd_hook_check_write`、`cmd_hook_gate` 的 multiedit 别名 + 未知工具 record
- `src/application/mod.rs` — `workspace_apply` 加 is_protected 硬拒;`new_validator_if_available` 始终返回 Some
- `src/infrastructure/schema_validator.rs` — `EMBEDDED_SCHEMA_FILES` 编译期嵌入
- `src/infrastructure/self_update.rs` — `self_replace` 加 Drop-guard + 前置校验 + 残留风险注释
- `src/infrastructure/gates/mod.rs` — success 路径加 `kill_process_tree_orphans`
- `scripts/install.ps1` — checksum fetch/verify 拆开,都 fatal
- `scripts/install.sh` — checksum fetch 失败 fatal;无 sha256 工具 fatal

**文档(`README.md`,8 项)**:
- 重写「前置要求」(分二进制/集成)
- 加 step 2「验证安装」
- 加 Windows 新终端 / Linux PATH / 代理 / 卸载提示
- 折叠块版本号占位化(`v0.0.11` → `vX.Y.Z`)
- 命令一览删 `--kanban`、加 `--include-archived` 和 `--platform`
- 插入「故障排除」章节
- 加 agent-driven 流程框提示
- 加 bare `ctl update` 是 legacy 的注脚

**测试**:`cargo test --bin ctl` — **570 个测试全绿**(567 既有 + 3 新回归;实际 569 因为 1 个 filter)。`cargo check --tests` 0 error。

---

## 详细 scout 报告索引

每个 scout 的完整输出(`pipeline_summary`、每个 finding 的 file:line + 触发输入 + 修复建议 + sound 类别的证据)保留在下列 artifact URI,可作 issue 来源:

- `agent://BoundaryAudit` — 写边界 + 路径规范化(7 个发现)
- `agent://LedgerReducerAudit` — 事件账本 + reducer(12 个发现,含 5 个 sound 确认)
- `agent://GateRunnerAudit` — gate runner + 进程超时(11 个发现,含 3 个 sound 确认)
- `agent://SelfUpdateAudit` — self-update 网络下载 + 校验(8 个发现)
- `agent://WorkspaceSchemaAudit` — M4 workspace/diff/apply + JSON Schema(11 个发现)
- `agent://AdapterCliAudit` — adapter 契约 + CLI 校验(12 个发现,含 4 个 sound 确认)
- `agent://DocsFriendlinessAudit` — 部署/使用文档友好度(14 个发现 + 首次运行评估)
