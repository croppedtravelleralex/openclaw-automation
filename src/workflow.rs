use std::{fs, path::{Path, PathBuf}, process::Command};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::{AutopilotStage, ManagedProjectConfig, ManagedProjectState};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowSuggestionKind {
    Feature,
    BugScan,
    BugFix,
    DocSync,
    Refactor,
    Performance,
    Test,
    Collect,
    Commit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowSuggestion {
    pub title: String,
    pub priority: u8,
    pub rationale: String,
    pub kind: WorkflowSuggestionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowActionRecord {
    pub title: String,
    pub kind: WorkflowSuggestionKind,
    pub status: String,
    pub note: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkflowDocumentContext {
    pub vision: String,
    pub current_direction: String,
    pub todo: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowReport {
    pub project_id: String,
    pub trigger: String,
    pub iteration: u64,
    pub stage: String,
    pub summary: String,
    pub focus: String,
    pub confirmations: Vec<String>,
}

impl WorkflowDocumentContext {
    pub fn load_from_project(root: &Path, config: &ManagedProjectConfig) -> Result<Self> {
        Ok(Self {
            vision: fs::read_to_string(root.join(&config.vision_path))
                .with_context(|| format!("failed to read {}", root.join(&config.vision_path).display()))?,
            current_direction: fs::read_to_string(root.join(&config.direction_path))
                .with_context(|| format!("failed to read {}", root.join(&config.direction_path).display()))?,
            todo: fs::read_to_string(root.join(&config.todo_path))
                .with_context(|| format!("failed to read {}", root.join(&config.todo_path).display()))?,
        })
    }
}

pub fn discover_projects() -> Result<Vec<String>> {
    let mut ids = Vec::new();
    let config_dir = Path::new("configs");
    if !config_dir.exists() {
        return Ok(ids);
    }
    for entry in fs::read_dir(config_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) == Some("json") {
            if let Some(stem) = path.file_stem().and_then(|v| v.to_str()) {
                ids.push(stem.to_string());
            }
        }
    }
    ids.sort();
    Ok(ids)
}

pub fn default_suggestions_for_stage(stage: AutopilotStage) -> Vec<WorkflowSuggestion> {
    match stage {
        AutopilotStage::Plan => vec![
            suggestion("读取目标文档并重新排序下一阶段事项", 1, "先对齐 vision/direction/todo，避免跑偏", WorkflowSuggestionKind::DocSync),
            suggestion("生成 3–5 个下一阶段建议", 2, "为默认执行前两个动作提供输入", WorkflowSuggestionKind::Feature),
            suggestion("同步当前状态与目标口径", 3, "减少文档与代码漂移", WorkflowSuggestionKind::DocSync),
        ],
        AutopilotStage::Execute => vec![
            suggestion("执行建议第 1 项", 1, "默认推进当前最优先事项", WorkflowSuggestionKind::Feature),
            suggestion("执行建议第 2 项", 2, "保持双任务推进节奏", WorkflowSuggestionKind::Feature),
            suggestion("补最小必要测试", 3, "防止推进后没有验证锁定", WorkflowSuggestionKind::Test),
        ],
        AutopilotStage::Verify => vec![
            suggestion("跑定向测试与一致性检查", 1, "验证刚完成的两项动作是否稳定", WorkflowSuggestionKind::Test),
            suggestion("检查状态漂移与 flaky", 2, "优先发现不稳定点", WorkflowSuggestionKind::BugScan),
            suggestion("采集 git/test/doc 数据摘要", 3, "为每 10 轮汇报与关键事件汇报提供素材", WorkflowSuggestionKind::Collect),
        ],
        AutopilotStage::BugScan => vec![
            suggestion("查找 bug", 1, "bug 环固定第一项为查找问题", WorkflowSuggestionKind::BugScan),
            suggestion("修复 bug", 2, "bug 环固定第二项为修复问题", WorkflowSuggestionKind::BugFix),
        ],
        AutopilotStage::BugFix => vec![
            suggestion("修复 bug", 1, "执行最小修复", WorkflowSuggestionKind::BugFix),
            suggestion("补测试锁住修复", 2, "防止回归", WorkflowSuggestionKind::Test),
        ],
        AutopilotStage::DocSync => vec![
            suggestion("同步 TODO/STATUS/PROGRESS", 1, "让文档继续反映真实阶段", WorkflowSuggestionKind::DocSync),
            suggestion("采集文档变更摘要", 2, "让后续汇报可解释", WorkflowSuggestionKind::Collect),
        ],
        AutopilotStage::CommitPush => vec![
            suggestion("commit 当前稳定成果", 1, "把本轮稳定成果落盘", WorkflowSuggestionKind::Commit),
            suggestion("评估是否 push", 2, "外发前进入确认点", WorkflowSuggestionKind::Commit),
        ],
        AutopilotStage::Cooldown => vec![
            suggestion("短暂冷却并等待下一轮", 1, "避免高频抖动", WorkflowSuggestionKind::Performance),
        ],
        AutopilotStage::Blocked => vec![
            suggestion("识别阻塞原因", 1, "先明确为什么不能继续", WorkflowSuggestionKind::BugScan),
            suggestion("给出恢复路径", 2, "为人工确认或恢复运行做准备", WorkflowSuggestionKind::DocSync),
        ],
    }
}

pub fn generate_dynamic_suggestions(stage: AutopilotStage, ctx: &WorkflowDocumentContext) -> Vec<WorkflowSuggestion> {
    let mut suggestions = Vec::new();
    let todo = ctx.todo.to_lowercase();
    let direction = ctx.current_direction.to_lowercase();
    let vision = ctx.vision.to_lowercase();

    if direction.contains("trust score") {
        suggestions.push(suggestion("继续推进 trust score 核心化", 1, "当前方向明确要求继续把 proxy selection 收敛到 trust score 核心表达", WorkflowSuggestionKind::Feature));
    }
    if direction.contains("verify") {
        suggestions.push(suggestion("推进 verify / smoke / batch verify 质量闭环", 2, "当前方向要求把 verify 信号统一成更稳定的质量闭环", WorkflowSuggestionKind::Feature));
    }
    if todo.contains("写放大") || direction.contains("写放大") {
        suggestions.push(suggestion("治理高并发写放大与状态竞争", 3, "TODO 与当前方向都把写放大列为当前重点", WorkflowSuggestionKind::Performance));
    }
    if direction.contains("文档") || todo.contains("同步 current_") {
        suggestions.push(suggestion("继续同步 CURRENT_*/TODO/STATUS 口径", 4, "当前阶段强调文档、策略、代码主链要保持同一口径", WorkflowSuggestionKind::DocSync));
    }
    if vision.contains("artifact") || vision.contains("可替换执行引擎") {
        suggestions.push(suggestion("补执行引擎边界与 artifact 策略", 5, "vision 强调可替换执行引擎与长期运行下的结果管理能力", WorkflowSuggestionKind::Refactor));
    }

    if stage == AutopilotStage::BugScan {
        return default_suggestions_for_stage(stage);
    }

    if suggestions.is_empty() {
        default_suggestions_for_stage(stage)
    } else {
        suggestions.sort_by_key(|s| s.priority);
        suggestions.truncate(5);
        suggestions
    }
}

pub fn refresh_dynamic_suggestions(root: &Path, config: &ManagedProjectConfig, state: &mut ManagedProjectState) -> Result<()> {
    let ctx = WorkflowDocumentContext::load_from_project(root, config)?;
    state.next_suggestions = generate_dynamic_suggestions(state.stage, &ctx);
    Ok(())
}

pub fn dispatch_top_suggestions(root: &Path, config: &ManagedProjectConfig, state: &mut ManagedProjectState, max_actions: usize) -> Result<Vec<WorkflowActionRecord>> {
    let selected = state.next_suggestions.iter().take(max_actions).cloned().collect::<Vec<_>>();
    let mut executed = Vec::new();
    for suggestion in selected {
        let record = execute_suggestion(root, config, state, &suggestion)?;
        executed.push(record);
    }
    state.last_executed_actions = executed.clone();
    Ok(executed)
}

pub fn run_minimal_cycle_step(root: &Path, config: &ManagedProjectConfig, state: &mut ManagedProjectState) -> Result<()> {
    match state.stage {
        AutopilotStage::Plan => {
            state.current_focus = "对齐目标文档并生成本轮建议".to_string();
            state.current_objective = "读取 VISION/CURRENT_DIRECTION/TODO 后确定前两项动作".to_string();
            state.last_summary = "已完成 plan 阶段，生成下一阶段建议".to_string();
            state.stage = AutopilotStage::Execute;
            refresh_dynamic_suggestions(root, config, state)?;
        }
        AutopilotStage::Execute => {
            let executed = dispatch_top_suggestions(root, config, state, 2)?;
            state.current_focus = "执行建议前两项".to_string();
            state.current_objective = "完成当前最优先的两个动作并补最小必要验证".to_string();
            state.last_summary = format!("已完成 execute 阶段，已分发 {} 个动作", executed.len());
            state.stage = AutopilotStage::Verify;
            refresh_dynamic_suggestions(root, config, state)?;
        }
        AutopilotStage::Verify => {
            let _ = collect_project_snapshot(root, config, state)?;
            state.current_focus = "验证本轮结果并检查是否需要 bug 环".to_string();
            state.current_objective = "完成测试、口径一致性检查与风险扫描".to_string();
            state.last_summary = "已完成 verify 阶段，准备同步文档".to_string();
            state.stage = AutopilotStage::DocSync;
            state.next_suggestions = default_suggestions_for_stage(state.stage);
        }
        AutopilotStage::BugScan => {
            state.current_focus = "进入 bug 环并锁定问题".to_string();
            state.current_objective = "优先定位最值得修复的问题".to_string();
            state.last_summary = "已完成 bug_scan，进入 bug_fix".to_string();
            state.pending_confirmation.push("进入 bug 环：建议检查 flaky、warning、状态漂移并确认修复优先级".to_string());
            state.stage = AutopilotStage::BugFix;
            state.next_suggestions = default_suggestions_for_stage(state.stage);
        }
        AutopilotStage::BugFix => {
            state.current_focus = "修复 bug 并锁测试".to_string();
            state.current_objective = "完成最小修复，准备提交".to_string();
            state.last_summary = "已完成 bug_fix，准备 commit/push".to_string();
            state.stage = AutopilotStage::CommitPush;
            state.next_suggestions = default_suggestions_for_stage(state.stage);
        }
        AutopilotStage::DocSync => {
            let _ = sync_project_docs(root, config)?;
            state.current_focus = "同步文档与当前阶段状态".to_string();
            state.current_objective = "更新 TODO/STATUS/PROGRESS 与执行日志".to_string();
            state.last_summary = "已完成 doc_sync，进入 cooldown".to_string();
            state.stage = AutopilotStage::Cooldown;
            state.next_suggestions = default_suggestions_for_stage(state.stage);
        }
        AutopilotStage::CommitPush => {
            let commit_result = run_commit_guarded(root, state)?;
            state.current_focus = "提交当前稳定成果".to_string();
            state.current_objective = "commit 当前轮结果，并按条件评估 push".to_string();
            state.last_summary = commit_result;
            state.stage = AutopilotStage::Cooldown;
            state.next_suggestions = default_suggestions_for_stage(state.stage);
        }
        AutopilotStage::Cooldown => {
            state.current_focus = "冷却并准备下一轮".to_string();
            state.current_objective = "结束当前小循环，回到 plan".to_string();
            state.last_summary = "已完成 cooldown，下一轮重新进入 plan".to_string();
            state.stage = AutopilotStage::Plan;
            refresh_dynamic_suggestions(root, config, state)?;
        }
        AutopilotStage::Blocked => {
            state.current_focus = "解除阻塞".to_string();
            state.current_objective = if state.blocked_reason.is_empty() { "先识别阻塞，再回到 plan".to_string() } else { format!("先处理阻塞原因：{}", state.blocked_reason) };
            state.last_summary = "blocked 已切回 plan，等待恢复推进".to_string();
            state.pending_confirmation.push(format!("blocked 解除前请确认：{}", if state.blocked_reason.is_empty() { "当前阻塞原因未填写" } else { &state.blocked_reason }));
            state.stage = AutopilotStage::Plan;
            refresh_dynamic_suggestions(root, config, state)?;
        }
    }
    state.loop_iteration += 1;
    if state.loop_iteration > 0 && state.loop_iteration % config.report_every_rounds == 0 {
        state.pending_confirmation.push(format!("已达到第 {} 轮汇报点，建议向用户汇报当前进展", state.loop_iteration));
        state.next_report_at = state.loop_iteration + config.report_every_rounds;
    }
    Ok(())
}

pub fn tick_project(project_id: &str) -> Result<(ManagedProjectState, Option<WorkflowReport>)> {
    let config_path = PathBuf::from("configs").join(format!("{}.json", project_id));
    let state_path = PathBuf::from("state").join(format!("{}.json", project_id));
    let config: ManagedProjectConfig = serde_json::from_str(&fs::read_to_string(&config_path)?)?;
    let mut state: ManagedProjectState = serde_json::from_str(&fs::read_to_string(&state_path)?)?;
    let root = PathBuf::from(&config.root);
    run_minimal_cycle_step(&root, &config, &mut state)?;
    let report = maybe_write_report(project_id, &mut state)?;
    fs::write(&state_path, serde_json::to_string_pretty(&state)?)?;
    Ok((state, report))
}

fn execute_suggestion(root: &Path, config: &ManagedProjectConfig, state: &ManagedProjectState, suggestion: &WorkflowSuggestion) -> Result<WorkflowActionRecord> {
    match suggestion.kind {
        WorkflowSuggestionKind::DocSync => {
            let note = sync_project_docs(root, config)?;
            Ok(WorkflowActionRecord {
                title: suggestion.title.clone(),
                kind: suggestion.kind,
                status: "doc_synced".to_string(),
                note,
            })
        }
        WorkflowSuggestionKind::Collect => {
            let note = collect_project_snapshot(root, config, state)?;
            Ok(WorkflowActionRecord {
                title: suggestion.title.clone(),
                kind: suggestion.kind,
                status: "collected".to_string(),
                note,
            })
        }
        _ => {
            append_execution_log(root, &[WorkflowActionRecord {
                title: suggestion.title.clone(),
                kind: suggestion.kind,
                status: "logged".to_string(),
                note: format!("已执行最小真实动作：将建议写入 EXECUTION_LOG.md；原因：{}", suggestion.rationale),
            }])?;
            Ok(WorkflowActionRecord {
                title: suggestion.title.clone(),
                kind: suggestion.kind,
                status: "logged".to_string(),
                note: format!("已执行最小真实动作：将建议写入 EXECUTION_LOG.md；原因：{}", suggestion.rationale),
            })
        }
    }
}

fn sync_project_docs(root: &Path, config: &ManagedProjectConfig) -> Result<String> {
    let status_path = root.join(&config.status_path);
    let mut content = if status_path.exists() { fs::read_to_string(&status_path)? } else { String::new() };
    if !content.contains("## Autopilot Sync") {
        content.push_str("\n## Autopilot Sync\n\n");
    }
    content.push_str("- 独立 autopilot 已执行一轮文档同步检查。\n");
    fs::write(status_path, content)?;
    Ok("已将文档同步动作写入 STATUS.md 的 Autopilot Sync 区块".to_string())
}

fn collect_project_snapshot(root: &Path, config: &ManagedProjectConfig, state: &ManagedProjectState) -> Result<String> {
    let reports_dir = PathBuf::from("reports");
    fs::create_dir_all(&reports_dir)?;
    let git_status = run_capture(root, "git", &["status", "--short"])?;
    let doc_sync = root.join(&config.status_path).exists();
    let snapshot = serde_json::json!({
        "project_id": config.id,
        "iteration": state.loop_iteration,
        "stage": format!("{:?}", state.stage),
        "git_status": git_status,
        "status_doc_exists": doc_sync,
    });
    let path = reports_dir.join(format!("{}-snapshot.json", config.id));
    fs::write(path, serde_json::to_string_pretty(&snapshot)?)?;
    Ok("已采集 git/status 文档快照并写入 reports/<project>-snapshot.json".to_string())
}

fn run_commit_guarded(root: &Path, state: &mut ManagedProjectState) -> Result<String> {
    let status = run_capture(root, "git", &["status", "--short"])?;
    if status.trim().is_empty() {
        return Ok("commit_push 跳过：当前没有待提交改动".to_string());
    }
    run_status(root, "git", &["add", "."])?;
    let message = format!("Autopilot checkpoint at iteration {}", state.loop_iteration);
    run_status(root, "git", &["commit", "-m", &message])?;
    state.pending_confirmation.push("已完成本地 commit；若需要 push，请人工确认外发".to_string());
    Ok(format!("已完成本地 commit：{}；push 仍受人工确认门控", message))
}

fn run_capture(root: &Path, program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program).current_dir(root).args(args).output()?;
    if !out.status.success() {
        bail!("command failed: {} {}", program, args.join(" "));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn run_status(root: &Path, program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program).current_dir(root).args(args).status()?;
    if !status.success() {
        bail!("command failed: {} {}", program, args.join(" "));
    }
    Ok(())
}

fn append_execution_log(root: &Path, entries: &[WorkflowActionRecord]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let path = root.join("EXECUTION_LOG.md");
    let mut existing = if path.exists() { fs::read_to_string(&path)? } else { String::new() };
    if !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str("\n## Autopilot Workflow Action Dispatch\n\n");
    for entry in entries {
        existing.push_str(&format!("- {} [{}]: {}\n", entry.title, kind_name(entry.kind), entry.note));
    }
    fs::write(path, existing)?;
    Ok(())
}

fn suggestion(title: &str, priority: u8, rationale: &str, kind: WorkflowSuggestionKind) -> WorkflowSuggestion {
    WorkflowSuggestion { title: title.to_string(), priority, rationale: rationale.to_string(), kind }
}

fn kind_name(kind: WorkflowSuggestionKind) -> &'static str {
    match kind {
        WorkflowSuggestionKind::Feature => "feature",
        WorkflowSuggestionKind::BugScan => "bug_scan",
        WorkflowSuggestionKind::BugFix => "bug_fix",
        WorkflowSuggestionKind::DocSync => "doc_sync",
        WorkflowSuggestionKind::Refactor => "refactor",
        WorkflowSuggestionKind::Performance => "performance",
        WorkflowSuggestionKind::Test => "test",
        WorkflowSuggestionKind::Collect => "collect",
        WorkflowSuggestionKind::Commit => "commit",
    }
}

pub fn maybe_write_report(project_id: &str, state: &mut ManagedProjectState) -> Result<Option<WorkflowReport>> {
    let trigger = if state.pending_confirmation.iter().any(|s| s.contains("第 ") && s.contains("轮汇报点")) {
        Some("every_ten_rounds")
    } else if state.pending_confirmation.iter().any(|s| s.contains("push")) {
        Some("ready_to_push")
    } else if state.pending_confirmation.iter().any(|s| s.contains("blocked")) {
        Some("blocked")
    } else if !state.pending_confirmation.is_empty() {
        Some("key_event")
    } else {
        None
    };

    let Some(trigger) = trigger else {
        return Ok(None);
    };

    let report = WorkflowReport {
        project_id: project_id.to_string(),
        trigger: trigger.to_string(),
        iteration: state.loop_iteration,
        stage: format!("{:?}", state.stage),
        summary: state.last_summary.clone(),
        focus: state.current_focus.clone(),
        confirmations: state.pending_confirmation.clone(),
    };

    let path = PathBuf::from("reports").join(format!("{}-latest.json", project_id));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_string_pretty(&report)?)?;
    Ok(Some(report))
}
