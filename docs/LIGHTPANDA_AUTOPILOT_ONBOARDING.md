# LIGHTPANDA_AUTOPILOT_ONBOARDING

## 目标

让 `project-autopilot` 以**受控接入**方式进入 `lightpanda-automation`，先承担低风险推进工作，不直接托管高风险主线改造。

## 当前策略

### 允许自动执行（白名单）

1. **verify / smoke / batch verify 质量闭环**
   - 执行动作：`cargo test -q`
   - 理由：低风险、可验证、能直接形成质量信号

2. **bug_fix 最小验证**
   - 执行动作：`cargo test -q`
   - 理由：先让 autopilot 参与修复后的验证，而不是直接大面积改代码

3. **DocSync / Collect**
   - 执行动作：内部 action
   - 理由：同步文档、采集状态、汇总快照，本身低风险

### 暂不自动执行（仅影子推进 / 记录计划）

1. **继续推进 trust score 核心化**
   - 当前只写入 `EXECUTION_LOG.md`
   - 理由：这是当前业务主链，返工成本高，先不让 autopilot 直接改核心策略代码

2. **高并发写放大 / 状态竞争治理**
3. **代理选择策略主链大改**
4. **数据库结构迁移 / 关键接口语义变更**

## 接入原则

- 先让 autopilot 在 `lightpanda-automation` 里承担：
  - 文档同步
  - 状态采集
  - 定向验证
  - 低风险影子推进
- 对高风险主线：
  - 先给建议
  - 先记录计划
  - 先进入 `shadow-plan`
  - 不直接自动重构核心逻辑

## 当前配置落点

- 配置文件：`configs/lightpanda-automation.json`
- 当前受控映射：
  - `action_commands.bug_fix -> cargo test -q`
  - `action_command_overrides[推进 verify / smoke / batch verify 质量闭环] -> cargo test -q`
  - `action_command_overrides[继续推进 trust score 核心化] -> shadow-plan 日志记录`

## 下一阶段放权条件

只有满足以下条件，才建议放开更深层 feature 自动执行：

1. autopilot 在真实项目里继续稳定运行多轮
2. verify / smoke / batch verify 结果足够稳定
3. 文档口径已同步，不再误导 action generation
4. 能在 feature 分支或 shadow 分支安全落地试改
