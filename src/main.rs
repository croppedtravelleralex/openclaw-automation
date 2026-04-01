use std::{env, fs, path::Path, time::Duration};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

mod workflow;

use workflow::{discover_projects, render_report_message, tick_project, WorkflowActionRecord, WorkflowSuggestion};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReportPolicy {
    EveryTenRounds,
    KeyEventsOnly,
    Hybrid,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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
    consecutive_failures: u32,
    last_error: String,
    last_failure_at_ms: u64,
    cooldown_until_ms: u64,
    paused: bool,
    manual_hold_reason: String,
    last_error_category: String,
    recovery_hint: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("init") => init_skeleton(),
        Some("show") => show_example(args.get(2).map(|s| s.as_str()).unwrap_or("lightpanda-automation")),
        Some("status") => show_status(args.get(2).map(|s| s.as_str()).unwrap_or("lightpanda-automation")),
        Some("list-projects") => {
            for id in discover_projects()? {
                println!("{}", id);
            }
            Ok(())
        }
        Some("tick") => {
            let project_id = args.get(2).map(|s| s.as_str()).unwrap_or("lightpanda-automation");
            let (state, report) = tick_project(project_id)?;
            println!("autopilot tick ok: project={}, stage={:?}, iteration={}", state.project_id, state.stage, state.loop_iteration);
            if let Some(report) = report {
                println!("autopilot report emitted: trigger={}, iteration={}\n{}", report.trigger, report.iteration, render_report_message(&report));
            }
            Ok(())
        }
        Some("pause") => update_manual_control(args.get(2).map(|s| s.as_str()).unwrap_or("lightpanda-automation"), ManualControlCommand::Pause),
        Some("resume") => update_manual_control(args.get(2).map(|s| s.as_str()).unwrap_or("lightpanda-automation"), ManualControlCommand::Resume),
        Some("hold") => update_manual_control_with_reason(args.get(2).map(|s| s.as_str()).unwrap_or("lightpanda-automation"), ManualControlCommand::Hold, &args[3..]),
        Some("unhold") => update_manual_control(args.get(2).map(|s| s.as_str()).unwrap_or("lightpanda-automation"), ManualControlCommand::Unhold),
        Some("daemon") => run_daemon(&args[2..]).await,
        Some("install-cron") => install_cron(&args[2..]),
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
        consecutive_failures: 0,
        last_error: String::new(),
        last_failure_at_ms: 0,
        cooldown_until_ms: 0,
        paused: false,
        manual_hold_reason: String::new(),
        last_error_category: String::new(),
        recovery_hint: String::new(),
    };

    write_json("configs/lightpanda-automation.json", &config)?;
    write_json("state/lightpanda-automation.json", &state)?;
    write_docs()?;
    println!("project-autopilot initialized");
    Ok(())
}

fn show_example(project_id: &str) -> Result<()> {
    let config = fs::read_to_string(format!("configs/{}.json", project_id))
        .with_context(|| format!("missing configs/{}.json", project_id))?;
    let state = fs::read_to_string(format!("state/{}.json", project_id))
        .with_context(|| format!("missing state/{}.json", project_id))?;
    println!("== config ==\n{}\n\n== state ==\n{}", config, state);
    Ok(())
}

fn show_status(project_id: &str) -> Result<()> {
    let state = load_state(project_id)?;
    println!("project: {}", state.project_id);
    println!("stage: {:?}", state.stage);
    println!("iteration: {}", state.loop_iteration);
    println!("paused: {}", state.paused);
    println!("manual_hold_reason: {}", if state.manual_hold_reason.is_empty() { "<none>" } else { &state.manual_hold_reason });
    println!("consecutive_failures: {}", state.consecutive_failures);
    println!("last_error_category: {}", if state.last_error_category.is_empty() { "<none>" } else { &state.last_error_category });
    println!("recovery_hint: {}", if state.recovery_hint.is_empty() { "<none>" } else { &state.recovery_hint });
    println!("last_summary: {}", state.last_summary);
    println!("current_focus: {}", state.current_focus);
    println!("current_objective: {}", state.current_objective);
    println!("next_report_at: {}", state.next_report_at);
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum ManualControlCommand {
    Pause,
    Resume,
    Hold,
    Unhold,
}

fn load_state(project_id: &str) -> Result<ManagedProjectState> {
    let path = format!("state/{}.json", project_id);
    let content = fs::read_to_string(&path).with_context(|| format!("missing {}", path))?;
    Ok(serde_json::from_str(&content)?)
}

fn save_state(project_id: &str, state: &ManagedProjectState) -> Result<()> {
    write_json(&format!("state/{}.json", project_id), state)
}

fn update_manual_control(project_id: &str, cmd: ManualControlCommand) -> Result<()> {
    update_manual_control_internal(project_id, cmd, None)
}

fn update_manual_control_with_reason(project_id: &str, cmd: ManualControlCommand, rest: &[String]) -> Result<()> {
    let reason = if rest.is_empty() { None } else { Some(rest.join(" ")) };
    update_manual_control_internal(project_id, cmd, reason)
}

fn update_manual_control_internal(project_id: &str, cmd: ManualControlCommand, reason: Option<String>) -> Result<()> {
    let mut state = load_state(project_id)?;
    match cmd {
        ManualControlCommand::Pause => {
            state.paused = true;
            state.last_summary = "已手动暂停 autopilot".to_string();
            state.current_focus = "等待人工恢复".to_string();
            state.current_objective = "暂停自动推进，直到收到 resume".to_string();
        }
        ManualControlCommand::Resume => {
            state.paused = false;
            state.cooldown_until_ms = 0;
            state.last_summary = "已恢复 autopilot 自动推进".to_string();
            state.current_focus = "恢复自动执行".to_string();
            state.current_objective = "从当前阶段继续推进".to_string();
        }
        ManualControlCommand::Hold => {
            state.paused = true;
            state.manual_hold_reason = reason.unwrap_or_else(|| "manual hold".to_string());
            state.last_summary = format!("已进入 manual hold：{}", state.manual_hold_reason);
            state.current_focus = "等待人工解除 hold".to_string();
            state.current_objective = "保留现场，禁止自动推进".to_string();
        }
        ManualControlCommand::Unhold => {
            state.paused = false;
            state.manual_hold_reason.clear();
            state.cooldown_until_ms = 0;
            state.last_summary = "已解除 manual hold".to_string();
            state.current_focus = "恢复自动执行".to_string();
            state.current_objective = "从当前阶段继续推进".to_string();
        }
    }
    save_state(project_id, &state)?;
    println!("manual control updated: project={}, paused={}, hold_reason={}", project_id, state.paused, if state.manual_hold_reason.is_empty() { "<none>" } else { &state.manual_hold_reason });
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
"#,
    )?;
    fs::write(
        "README.md",
        r#"# project-autopilot

独立于业务项目的自动推进执行器。

## 命令

- `cargo run -- init`
- `cargo run -- show <project-id>`
- `cargo run -- status <project-id>`
- `cargo run -- list-projects`
- `cargo run -- tick <project-id>`
- `cargo run -- pause <project-id>` / `resume <project-id>`
- `cargo run -- hold <project-id> [reason...]` / `unhold <project-id>`
- `cargo run -- daemon <project-id> [--interval-seconds N] [--ticks M]`

## 当前能力

- 多项目配置发现
- 独立 workflow 内核
- 周期 tick / daemon
- pause / resume / manual hold 控制面
- status 运行态查询
- 每 10 次 + 关键事件报告
- 文档同步动作
- 数据采集动作
- commit 本地门控（push 保留人工确认）
"#,
    )?;
    Ok(())
}

fn print_help() {
    println!("project-autopilot usage:");
    println!("  project-autopilot init");
    println!("  project-autopilot show <project-id>");
    println!("  project-autopilot status <project-id>");
    println!("  project-autopilot list-projects");
    println!("  project-autopilot tick <project-id>");
    println!("  project-autopilot pause <project-id>");
    println!("  project-autopilot resume <project-id>");
    println!("  project-autopilot hold <project-id> [reason...]");
    println!("  project-autopilot unhold <project-id>");
    println!("  project-autopilot daemon <project-id> [--interval-seconds N] [--ticks M]");
    println!("  project-autopilot install-cron <project-id> [--interval-seconds N]");
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
    println!("autopilot daemon start: project={}, interval={}s, ticks={}", project_id, interval_seconds, max_ticks);
    let mut executed = 0usize;
    loop {
        let (state, report) = tick_project(project_id)?;
        executed += 1;
        println!("autopilot daemon tick {} ok: project={}, stage={:?}, iteration={}", executed, state.project_id, state.stage, state.loop_iteration);
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


fn install_cron(args: &[String]) -> Result<()> {
    let project_id = args.first().map(|s| s.as_str()).unwrap_or("lightpanda-automation");
    let mut interval_seconds: u64 = 600;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--interval-seconds" => {
                let value = args.get(i + 1).ok_or_else(|| anyhow!("missing value for --interval-seconds"))?;
                interval_seconds = value.parse::<u64>()?;
                i += 2;
            }
            other => bail!("unknown install-cron arg: {}", other),
        }
    }
    let minutes = std::cmp::max(1, interval_seconds / 60);
    let cron_line = format!("*/{} * * * * cd /root/SelfMadeprojects/project-autopilot && cargo run -- daemon {} --interval-seconds {} --ticks 1 >> reports/cron.log 2>&1
", minutes, project_id, interval_seconds);
    fs::write("reports/cron-example.txt", &cron_line)?;
    println!("cron example written to reports/cron-example.txt\n{}", cron_line);
    Ok(())
}
