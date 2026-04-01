# project-autopilot

独立于业务项目的自动推进执行器。

## 命令

- `cargo run -- init` 初始化示例配置与状态
- `cargo run -- show` 查看示例配置与状态
- `cargo run -- tick <project-id>` 执行一次独立 autopilot tick

## 目标

- 托管 SelfMadeprojects 下的一个或多个项目
- 自动生成建议、默认执行前两个
- 采集数据、每 10 次汇报、关键事件提前汇报
- 将确认点从“每一步都问”压缩到关键决策点
