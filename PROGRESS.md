# PROGRESS

## 已真正落地的能力

1. **Autopilot 状态机**
   - Plan / Execute / Verify / BugScan / BugFix / DocSync / CommitPush / Cooldown / Blocked

2. **失败治理**
   - `consecutive_failures`
   - `last_error_category`
   - `recovery_hint`
   - `cooldown_until_ms`
   - blocked 触发与提示

3. **人工控制面**
   - `pause`
   - `resume`
   - `hold`
   - `unhold`
   - `status`

4. **执行器能力**
   - 文档同步
   - 快照采集
   - cargo test
   - git commit check
   - kind/title 级命令映射

5. **ActionPlan DSL（最小骨架）**
   - `ActionPlan`
   - `ActionNode`
   - `ActionExecutor`
   - `ActionFailurePolicy`
   - `ActionVerifySpec`
   - `RetryPolicy`
   - `ActionRollbackSpec`

6. **测试覆盖**
   - 当前共 **27 个测试通过**
   - 已覆盖文件级集成链路
