use std::{env, fs, path::Path, time::Duration};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

mod workflow;

use tokio::time::sleep;
use workflow::{tick_project, WorkflowActionRecord, WorkflowSuggestion};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReportPolicy {
    EveryTenRounds,
    KeyEventsOnly,
    Hybrid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConfirmationPolicy {
    ArchitectureDecision,
    ExternalPush,
    DestructiveChange,
    HeavyInstall,
    RepeatedFailure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManagedProjectConfig {
    id: String,
    root: String,
    enabled: bool,
    default_execute: bool,
    collect_data: bool,
    report_every_rounds: u64,
    report_policy: ReportPolicy,
    confirmation_points: Vec<ConfirmationPolicy>,
    vision_path: String,
    direction_path: String,
    todo_path: String,
    status_path: String,
    progress_path: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AutopilotStage {
    Plan,
    Execute,
    Verify,
    BugScan,
    BugFix,
    DocSync,
    CommitPush,
    Cooldown,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManagedProjectState {
    project_id: String,
    loop_iteration: u64,
    stage: AutopilotStage,
    default_execute: bool,
    collect_data: bool,
    last_summary: String,
    next_report_at: u64,
    blocked_reason: String,
    pending_confirmation: Vec<String>,
    current_focus: String,
    current_objective: String,
    next_suggestions: Vec<WorkflowSuggestion>,
    last_executed_actions: Vec<WorkflowActionRecord>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("init") => init_skeleton(),
        Some("show") => show_example(),
        Some("tick") => {
            let project_id = args.get(2).map(|s| s.as_str()).unwrap_or("lightpanda-automation");
            let (state, report) = tick_project(project_id)?;
            println!("autopilot tick ok: project={}, stage={:?}, iteration={}", state.project_id, state.stage, state.loop_iteration);
            if let Some(report) = report {
                println!("autopilot report emitted: trigger={}, iteration={}", report.trigger, report.iteration);
            }
            Ok(())
        }
        Some("daemon") => run_daemon(&args[2..]).await,
        _ => {
            print_help();
            Ok(())
        }
    }
}

fn init_skeleton() -> Result<()> {
    let config = ManagedProjectConfig {
        id: "lightpanda-automation".to_string(),
        root: "/root/SelfMadeprojects/lightpanda-automation".to_string(),
        enabled: true,
        default_execute: true,
        collect_data: true,
        report_every_rounds: 10,
        report_policy: ReportPolicy::Hybrid,
        confirmation_points: vec![
            ConfirmationPolicy::ArchitectureDecision,
            ConfirmationPolicy::ExternalPush,
            ConfirmationPolicy::DestructiveChange,
            ConfirmationPolicy::HeavyInstall,
            ConfirmationPolicy::RepeatedFailure,
        ],
        vision_path: "VISION.md".to_string(),
        direction_path: "CURRENT_DIRECTION.md".to_string(),
        todo_path: "TODO.md".to_string(),
        status_path: "STATUS.md".to_string(),
        progress_path: "PROGRESS.md".to_string(),
    };
    let state = ManagedProjectState {
        project_id: "lightpanda-automation".to_string(),
        loop_iteration: 0,
        stage: AutopilotStage::Plan,
        default_execute: true,
        collect_data: true,
        last_summary: "独立 autopilot 尚未开始运行".to_string(),
        next_report_at: 10,
        blocked_reason: String::new(),
        pending_confirmation: Vec::new(),
        current_focus: "等待进入 plan".to_string(),
        current_objective: "初始化独立 autopilot 内核".to_string(),
        next_suggestions: Vec::new(),
        last_executed_actions: Vec::new(),
    };

    write_json("configs/lightpanda-automation.json", &config)?;
    write_json("state/lightpanda-automation.json", &state)?;
    write_docs()?;
    println!("project-autopilot initialized");
    Ok(())
}

fn show_example() -> Result<()> {
    let config = fs::read_to_string("configs/lightpanda-automation.json")
        .context("missing configs/lightpanda-automation.json, run `project-autopilot init` first")?;
    let state = fs::read_to_string("state/lightpanda-automation.json")
        .context("missing state/lightpanda-automation.json, run `project-autopilot init` first")?;
    println!("== config ==\n{}\n\n== state ==\n{}", config, state);
    Ok(())
}

fn write_json(path: &str, value: &impl Serialize) -> Result<()> {
    let path = Path::new(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}

fn write_docs() -> Result<()> {
    fs::write(
        "docs/AUTOPILOT_DESIGN.md",
        r#"# AUTOPILOT_DESIGN.md

## 目标

把自动推进从业务项目中独立出来，做成 SelfMadeprojects 下的独立程序，支持：

1. 多项目托管
2. 默认执行模式
3. 数据采集模式
4. 每 10 次汇报 + 关键事件提前汇报
5. 用户确认点控制
6. 自动阶段推进

## 核心模式

- **默认执行**：自动生成建议并默认执行前两个
- **采集模式**：优先采集测试/日志/git/warning/性能数据，不急着做重修改
- **混合模式**：推进与采集同时存在

## 汇报策略

- 正常情况：每 10 次汇报一次
- 提前汇报事件：
  - blocked
  - 连续失败
  - 准备 push
  - 高风险安装
  - 架构分叉

## 用户确认点

默认仅在以下情况确认：
- 架构决策
- 外部 push
- 破坏性修改
- 重依赖安装
- 连续失败过多

## 下一步

1. 接状态机内核
2. 接多项目发现器
3. 接真实建议器
4. 接数据采集器
5. 接汇报器
"#,
    )?;
    fs::write(
        "README.md",
        r#"# project-autopilot

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
"#,
    )?;
    Ok(())
}

fn print_help() {
    println!("project-autopilot usage:");
    println!("  project-autopilot init   Initialize autopilot skeleton/config/state");
    println!("  project-autopilot show   Show example config/state");
    println!("  project-autopilot tick <project-id>   Execute one autopilot workflow tick");
    println!("  project-autopilot daemon <project-id> [--interval-seconds N] [--ticks M]   Run periodic autopilot ticks");
}

async fn run_daemon(args: &[String]) -> Result<()> {
    let project_id = args.first().map(|s| s.as_str()).unwrap_or("lightpanda-automation");
    let mut interval_seconds: u64 = 300;
    let mut max_ticks: usize = 0;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--interval-seconds" => {
                let value = args.get(i + 1).ok_or_else(|| anyhow!("missing value for --interval-seconds"))?;
                interval_seconds = value.parse::<u64>()?;
                i += 2;
            }
            "--ticks" => {
                let value = args.get(i + 1).ok_or_else(|| anyhow!("missing value for --ticks"))?;
                max_ticks = value.parse::<usize>()?;
                i += 2;
            }
            other => bail!("unknown daemon arg: {}", other),
        }
    }
    if interval_seconds == 0 {
        bail!("--interval-seconds must be > 0");
    }
    println!("autopilot daemon start: project={}, interval={}s, ticks={}", project_id, interval_seconds, max_ticks);
    let mut executed = 0usize;
    loop {
        let (state, report) = tick_project(project_id)?;
        executed += 1;
        println!(
            "autopilot daemon tick {} ok: project={}, stage={:?}, iteration={}",
            executed, state.project_id, state.stage, state.loop_iteration
        );
        if let Some(report) = report {
            println!("autopilot report emitted: trigger={}, iteration={}", report.trigger, report.iteration);
        }
        if !state.pending_confirmation.is_empty() {
            println!("pending confirmations: {:?}", state.pending_confirmation);
        }
        if max_ticks > 0 && executed >= max_ticks {
            println!("autopilot daemon completed requested ticks");
            break;
        }
        sleep(Duration::from_secs(interval_seconds)).await;
    }
    Ok(())
}
