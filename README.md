# project-autopilot

一个独立于业务项目的 **自动推进执行器**。目标不是做“定时跑脚本”，而是做一个能 **读项目状态、生成建议、执行动作、失败治理、人工接管、持续汇报** 的 autopilot 内核。

## 当前能力

- **项目状态机**：`Plan / Execute / Verify / BugScan / BugFix / DocSync / CommitPush / Cooldown / Blocked`
- **失败治理**：连续失败计数、错误分类、恢复建议、指数 backoff、blocked
- **人工接管**：`pause / resume / hold / unhold / status`
- **结构化执行**：`DocSync / Collect / Test / Commit`
- **ActionPlan DSL（最小骨架）**：
  - `ActionPlan / ActionNode / ActionExecutor / ActionFailurePolicy`
  - `verify / retry / rollback`
  - 当前支持 executor：`Shell / InternalDocSync / InternalCollect`
- **执行映射**：
  - `action_commands[kind]`
  - `action_command_overrides[title]`
  - 兼容层会自动转成最小 `ActionPlan`
- **文件级集成测试**：已覆盖 `config/state -> tick_project -> state持久化 -> ActionPlan执行`

## 命令

- `cargo run -- init`
- `cargo run -- show <project-id>`
- `cargo run -- status <project-id>`
- `cargo run -- tick <project-id>`
- `cargo run -- pause <project-id>` / `resume <project-id>`
- `cargo run -- hold <project-id> [reason...]` / `unhold <project-id>`
- `cargo run -- daemon <project-id> [--interval-seconds N] [--ticks M]`

## Action 命中优先级

1. `action_command_overrides[建议标题]`
2. `action_commands[kind]`
3. fallback 到 plan / internal action

## ActionPlan 当前最小能力

每个 `ActionNode` 当前支持：
- `executor`
- `verify`
- `retry`
- `rollback`
- `on_fail`

当前 verify 模式：
- `exit_code_zero`
- `stdout_contains`

## 当前定位

这已经不是 demo，而是一个 **可持续推进个人研发项目的 autopilot MVP+**。
下一阶段的主线是：
- 多节点 Action Graph
- edge / condition
- 更强恢复策略
- 多项目调度

## 已验证稳定性

- 已通过真实文件链路下 **36 轮** 连续 tick 稳定性测试
- 当前测试数：**28 个测试通过**
- 已验证 `config/state -> tick_project -> ActionPlan -> state持久化` 主干可长期循环
