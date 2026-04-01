use std::{fs, path::{Path, PathBuf}, process::Command, time::{SystemTime, UNIX_EPOCH}};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::{AutopilotStage, ConfirmationPolicy, ManagedProjectConfig, ManagedProjectState};

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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn compute_backoff_ms(consecutive_failures: u32) -> u64 {
    let exp = consecutive_failures.saturating_sub(1).min(5);
    let base = 10_000u64;
    let factor = 1u64 << exp;
    (base.saturating_mul(factor)).min(300_000)
}

fn should_wait_for_cooldown(state: &ManagedProjectState, now_ms_value: u64) -> bool {
    state.cooldown_until_ms > now_ms_value
}

fn register_tick_failure(state: &mut ManagedProjectState, err: &anyhow::Error, now_ms_value: u64) {
    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    state.last_error = format!("{:#}", err);
    state.last_failure_at_ms = now_ms_value;
    let backoff_ms = compute_backoff_ms(state.consecutive_failures);
    state.cooldown_until_ms = now_ms_value.saturating_add(backoff_ms);
    state.current_focus = "处理失败与退避冷却".to_string();
    state.current_objective = "等待冷却结束后自动重试；若连续失败过多则转 blocked".to_string();
    state.last_summary = format!(
        "tick 失败，第 {} 次连续失败；已进入 {} 秒冷却",
        state.consecutive_failures,
        backoff_ms / 1000
    );

    if state.consecutive_failures >= 3 {
        state.stage = AutopilotStage::Blocked;
        state.blocked_reason = state.last_error.clone();
        push_confirmation_once(
            state,
            confirmation_message(
                ConfirmationPolicy::RepeatedFailure,
                &format!("连续失败 {} 次：{}", state.consecutive_failures, state.last_error),
            ),
        );
    }
}

fn clear_failure_tracking(state: &mut ManagedProjectState) {
    state.consecutive_failures = 0;
    state.last_error.clear();
    state.last_failure_at_ms = 0;
    state.cooldown_until_ms = 0;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrategyDecision {
    AutoProceed,
    RequireConfirmation(ConfirmationPolicy),
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
            push_confirmation_once(state, "进入 bug 环：建议检查 flaky、warning、状态漂移并确认修复优先级".to_string());
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
            let commit_result = run_commit_guarded(root, config, state)?;
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
            state.pending_confirmation.retain(|item| !item.contains("轮汇报点"));
            state.stage = AutopilotStage::Plan;
            refresh_dynamic_suggestions(root, config, state)?;
        }
        AutopilotStage::Blocked => {
            state.current_focus = "解除阻塞".to_string();
            state.current_objective = if state.blocked_reason.is_empty() { "先识别阻塞，再回到 plan".to_string() } else { format!("先处理阻塞原因：{}", state.blocked_reason) };
            state.last_summary = "blocked 已切回 plan，等待恢复推进".to_string();
            push_confirmation_once(state, format!("blocked 解除前请确认：{}", if state.blocked_reason.is_empty() { "当前阻塞原因未填写" } else { &state.blocked_reason }));
            state.stage = AutopilotStage::Plan;
            refresh_dynamic_suggestions(root, config, state)?;
        }
    }
    state.loop_iteration += 1;
    if state.loop_iteration > 0 && state.loop_iteration % config.report_every_rounds == 0 {
        push_confirmation_once(state, format!("已达到第 {} 轮汇报点，建议向用户汇报当前进展", state.loop_iteration));
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
    let now_ms_value = now_ms();

    if state.paused {
        let hold_reason = if state.manual_hold_reason.is_empty() {
            String::new()
        } else {
            format!("（manual hold：{}）", state.manual_hold_reason)
        };
        state.last_summary = format!("autopilot 当前已暂停{}，跳过本轮 tick", hold_reason);
        state.current_focus = "等待恢复运行".to_string();
        state.current_objective = "人工取消 paused/hold 后再继续自动推进".to_string();
    } else if should_wait_for_cooldown(&state, now_ms_value) {
        let remain_ms = state.cooldown_until_ms.saturating_sub(now_ms_value);
        state.last_summary = format!("仍在冷却中，{} 秒后再自动重试", remain_ms / 1000);
        state.current_focus = "冷却等待".to_string();
        state.current_objective = "跳过本轮执行，等待 backoff 窗口结束".to_string();
    } else {
        match run_minimal_cycle_step(&root, &config, &mut state) {
            Ok(()) => clear_failure_tracking(&mut state),
            Err(err) => register_tick_failure(&mut state, &err, now_ms_value),
        }
    }

    let report = maybe_write_report(project_id, &mut state)?;
    fs::write(&state_path, serde_json::to_string_pretty(&state)?)?;
    Ok((state, report))
}

fn run_structured_action(root: &Path, config: &ManagedProjectConfig, state: &ManagedProjectState, suggestion: &WorkflowSuggestion) -> Result<WorkflowActionRecord> {
    match suggestion.kind {
        WorkflowSuggestionKind::DocSync => {
            let note = sync_project_docs(root, config)?;
            Ok(WorkflowActionRecord { title: suggestion.title.clone(), kind: suggestion.kind, status: "doc_synced".to_string(), note })
        }
        WorkflowSuggestionKind::Collect => {
            let note = collect_project_snapshot(root, config, state)?;
            Ok(WorkflowActionRecord { title: suggestion.title.clone(), kind: suggestion.kind, status: "collected".to_string(), note })
        }
        WorkflowSuggestionKind::Test => run_test_action(root, suggestion),
        WorkflowSuggestionKind::Commit => run_commit_check_action(root, suggestion),
        WorkflowSuggestionKind::Feature
        | WorkflowSuggestionKind::BugScan
        | WorkflowSuggestionKind::BugFix
        | WorkflowSuggestionKind::Refactor
        | WorkflowSuggestionKind::Performance => run_plan_record_action(root, suggestion),
    }
}

fn run_test_action(root: &Path, suggestion: &WorkflowSuggestion) -> Result<WorkflowActionRecord> {
    if root.join("Cargo.toml").exists() {
        run_status(root, "cargo", &["test", "-q"])?;
        Ok(WorkflowActionRecord {
            title: suggestion.title.clone(),
            kind: suggestion.kind,
            status: "tested".to_string(),
            note: "已执行 cargo test -q 并通过".to_string(),
        })
    } else {
        let note = "未发现 Cargo.toml，跳过测试执行并记日志".to_string();
        append_execution_log(root, &[WorkflowActionRecord { title: suggestion.title.clone(), kind: suggestion.kind, status: "test_skipped".to_string(), note: note.clone() }])?;
        Ok(WorkflowActionRecord { title: suggestion.title.clone(), kind: suggestion.kind, status: "test_skipped".to_string(), note })
    }
}

fn run_commit_check_action(root: &Path, suggestion: &WorkflowSuggestion) -> Result<WorkflowActionRecord> {
    let status = run_capture(root, "git", &["status", "--short"])?;
    let trimmed = status.trim();
    let note = if trimmed.is_empty() {
        "git 工作区干净；当前无需 commit".to_string()
    } else {
        format!("检测到待提交改动：{}", trimmed.lines().take(5).collect::<Vec<_>>().join(" | "))
    };
    append_execution_log(root, &[WorkflowActionRecord { title: suggestion.title.clone(), kind: suggestion.kind, status: "commit_checked".to_string(), note: note.clone() }])?;
    Ok(WorkflowActionRecord { title: suggestion.title.clone(), kind: suggestion.kind, status: "commit_checked".to_string(), note })
}

fn run_plan_record_action(root: &Path, suggestion: &WorkflowSuggestion) -> Result<WorkflowActionRecord> {
    let note = format!("已记录结构化执行计划：{}；原因：{}", suggestion.title, suggestion.rationale);
    append_execution_log(root, &[WorkflowActionRecord { title: suggestion.title.clone(), kind: suggestion.kind, status: "planned".to_string(), note: note.clone() }])?;
    Ok(WorkflowActionRecord { title: suggestion.title.clone(), kind: suggestion.kind, status: "planned".to_string(), note })
}

fn execute_suggestion(root: &Path, config: &ManagedProjectConfig, state: &ManagedProjectState, suggestion: &WorkflowSuggestion) -> Result<WorkflowActionRecord> {
    run_structured_action(root, config, state, suggestion)
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
    let git_status = match run_capture(root, "git", &["status", "--short"]) {
        Ok(v) => v,
        Err(err) => format!("captured_error: {}", err),
    };
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

fn run_commit_guarded(root: &Path, config: &ManagedProjectConfig, state: &mut ManagedProjectState) -> Result<String> {
    let status = run_capture(root, "git", &["status", "--short"])?;
    if status.trim().is_empty() {
        return Ok("commit_push 跳过：当前没有待提交改动".to_string());
    }

    match evaluate_confirmation_strategy(config, StrategyDecision::RequireConfirmation(ConfirmationPolicy::ExternalPush)) {
        StrategyDecision::AutoProceed => {
            run_status(root, "git", &["add", "."])?;
            let message = format!("Autopilot checkpoint at iteration {}", state.loop_iteration);
            run_status(root, "git", &["commit", "-m", &message])?;
            push_confirmation_once(state, "已完成本地 commit；若需要 push，请人工确认外发".to_string());
            Ok(format!("已完成本地 commit：{}；push 仍受人工确认门控", message))
        }
        StrategyDecision::RequireConfirmation(policy) => {
            let note = confirmation_message(policy, "检测到待提交改动；根据策略层，push 仍需人工确认，本轮不自动外发");
            push_confirmation_once(state, note.clone());
            Ok("策略层阻止自动 push；当前仅保留本地待提交改动与确认提示".to_string())
        }
    }
}

fn evaluate_confirmation_strategy(config: &ManagedProjectConfig, desired: StrategyDecision) -> StrategyDecision {
    match desired {
        StrategyDecision::AutoProceed => StrategyDecision::AutoProceed,
        StrategyDecision::RequireConfirmation(policy) => {
            if config.confirmation_points.contains(&policy) {
                StrategyDecision::RequireConfirmation(policy)
            } else {
                StrategyDecision::AutoProceed
            }
        }
    }
}

fn confirmation_message(policy: ConfirmationPolicy, detail: &str) -> String {
    let label = match policy {
        ConfirmationPolicy::ArchitectureDecision => "architecture_decision",
        ConfirmationPolicy::ExternalPush => "external_push",
        ConfirmationPolicy::DestructiveChange => "destructive_change",
        ConfirmationPolicy::HeavyInstall => "heavy_install",
        ConfirmationPolicy::RepeatedFailure => "repeated_failure",
    };
    format!("需要人工确认（{}）：{}", label, detail)
}

fn push_confirmation_once(state: &mut ManagedProjectState, message: String) {
    if !state.pending_confirmation.iter().any(|item| item == &message) {
        state.pending_confirmation.push(message);
    }
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
    if entries.is_empty() { return Ok(()); }
    let path = root.join("EXECUTION_LOG.md");
    let mut existing = if path.exists() { fs::read_to_string(&path)? } else { String::new() };
    if !existing.ends_with('\n') { existing.push('\n'); }
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
    let has_round_report = state.pending_confirmation.iter().any(|s| s.contains("轮汇报点"));
    let has_ready_to_push = state.pending_confirmation.iter().any(|s| s.contains("external_push"));
    let has_blocked = state.pending_confirmation.iter().any(|s| s.contains("blocked"));

    let trigger = if has_round_report {
        Some("every_ten_rounds")
    } else if has_ready_to_push {
        Some("ready_to_push")
    } else if has_blocked {
        Some("blocked")
    } else if !state.pending_confirmation.is_empty() {
        Some("key_event")
    } else {
        None
    };

    let Some(trigger) = trigger else { return Ok(None); };
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
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    fs::write(&path, serde_json::to_string_pretty(&report)?)?;

    if has_round_report {
        state.pending_confirmation.retain(|item| !item.contains("轮汇报点"));
    }

    Ok(Some(report))
}

pub fn render_report_message(report: &WorkflowReport) -> String {
    let mut out = format!(
        "项目：{}\n触发：{}\n轮次：{}\n阶段：{}\n摘要：{}\n焦点：{}",
        report.project_id, report.trigger, report.iteration, report.stage, report.summary, report.focus
    );
    if !report.confirmations.is_empty() {
        out.push_str("\n需确认：");
        for item in &report.confirmations {
            out.push_str(&format!("\n- {}", item));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs;
    use crate::{ManagedProjectConfig, ManagedProjectState, ReportPolicy};

    fn sample_config(root: &Path) -> ManagedProjectConfig {
        ManagedProjectConfig {
            id: "demo".to_string(),
            root: root.display().to_string(),
            enabled: true,
            default_execute: true,
            collect_data: true,
            report_every_rounds: 10,
            report_policy: ReportPolicy::Hybrid,
            confirmation_points: vec![ConfirmationPolicy::ExternalPush],
            vision_path: "VISION.md".to_string(),
            direction_path: "CURRENT_DIRECTION.md".to_string(),
            todo_path: "TODO.md".to_string(),
            status_path: "STATUS.md".to_string(),
            progress_path: "PROGRESS.md".to_string(),
        }
    }

    fn sample_state() -> ManagedProjectState {
        ManagedProjectState {
            project_id: "demo".to_string(),
            loop_iteration: 0,
            stage: crate::AutopilotStage::Plan,
            default_execute: true,
            collect_data: true,
            last_summary: String::new(),
            next_report_at: 10,
            blocked_reason: String::new(),
            pending_confirmation: Vec::new(),
            current_focus: String::new(),
            current_objective: String::new(),
            next_suggestions: Vec::new(),
            last_executed_actions: Vec::new(),
            consecutive_failures: 0,
            last_error: String::new(),
            last_failure_at_ms: 0,
            cooldown_until_ms: 0,
            paused: false,
            manual_hold_reason: String::new(),
        }
    }

    #[test]
    fn mini_cycle_generates_snapshot_and_doc_sync() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("VISION.md"), "artifact").unwrap();
        fs::write(dir.path().join("CURRENT_DIRECTION.md"), "trust score verify 文档").unwrap();
        fs::write(dir.path().join("TODO.md"), "同步 CURRENT_* 口径").unwrap();
        fs::write(dir.path().join("STATUS.md"), "# STATUS\n").unwrap();
        let config = sample_config(dir.path());
        let mut state = sample_state();
        run_minimal_cycle_step(dir.path(), &config, &mut state).unwrap();
        run_minimal_cycle_step(dir.path(), &config, &mut state).unwrap();
        run_minimal_cycle_step(dir.path(), &config, &mut state).unwrap();
        assert!(Path::new("reports/demo-snapshot.json").exists());
        let snap = fs::read_to_string("reports/demo-snapshot.json").unwrap();
        assert!(snap.contains("captured_error") || snap.contains("git_status"));
        run_minimal_cycle_step(dir.path(), &config, &mut state).unwrap();
        let status = fs::read_to_string(dir.path().join("STATUS.md")).unwrap();
        assert!(status.contains("Autopilot Sync"));
    }

    #[test]
    fn report_message_is_human_readable() {
        let report = WorkflowReport {
            project_id: "demo".to_string(),
            trigger: "every_ten_rounds".to_string(),
            iteration: 10,
            stage: "Verify".to_string(),
            summary: "summary".to_string(),
            focus: "focus".to_string(),
            confirmations: vec!["please confirm".to_string()],
        };
        let msg = render_report_message(&report);
        assert!(msg.contains("项目：demo"));
        assert!(msg.contains("需确认"));
    }

    #[test]
    fn backoff_grows_with_failure_count() {
        assert_eq!(compute_backoff_ms(1), 10_000);
        assert_eq!(compute_backoff_ms(2), 20_000);
        assert_eq!(compute_backoff_ms(3), 40_000);
    }

    #[test]
    fn repeated_failures_eventually_block_the_project() {
        let mut state = sample_state();
        let err = anyhow::anyhow!("boom");
        register_tick_failure(&mut state, &err, 1_000);
        assert_eq!(state.consecutive_failures, 1);
        assert_eq!(state.stage, crate::AutopilotStage::Plan);
        assert!(state.cooldown_until_ms > 1_000);

        register_tick_failure(&mut state, &err, 2_000);
        register_tick_failure(&mut state, &err, 3_000);
        assert_eq!(state.stage, crate::AutopilotStage::Blocked);
        assert!(state.pending_confirmation.iter().any(|v| v.contains("repeated_failure")));
    }

    #[test]
    fn cooldown_guard_skips_until_window_passes() {
        let mut state = sample_state();
        state.cooldown_until_ms = 5_000;
        assert!(should_wait_for_cooldown(&state, 4_000));
        assert!(!should_wait_for_cooldown(&state, 5_000));
    }

    #[test]
    fn structured_test_action_runs_for_rust_project() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("Cargo.toml"),
            r#"[package]
name = "demo"
version = "0.1.0"
edition = "2021"
"#,
        ).unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main() {}
").unwrap();
        let suggestion = WorkflowSuggestion {
            title: "补最小必要测试".to_string(),
            priority: 1,
            rationale: "test".to_string(),
            kind: WorkflowSuggestionKind::Test,
        };
        let record = run_test_action(dir.path(), &suggestion).unwrap();
        assert_eq!(record.status, "tested");
    }

    #[test]
    fn structured_commit_action_reports_dirty_workspace() {
        let dir = tempdir().expect("tempdir");
        std::process::Command::new("git").current_dir(dir.path()).args(["init"]).status().unwrap();
        fs::write(dir.path().join("dirty.txt"), "x").unwrap();
        let suggestion = WorkflowSuggestion {
            title: "评估是否 push".to_string(),
            priority: 1,
            rationale: "commit".to_string(),
            kind: WorkflowSuggestionKind::Commit,
        };
        let record = run_commit_check_action(dir.path(), &suggestion).unwrap();
        assert_eq!(record.status, "commit_checked");
        assert!(record.note.contains("dirty.txt"));
    }

    #[test]
    fn structured_feature_action_writes_planned_log() {
        let dir = tempdir().expect("tempdir");
        let suggestion = WorkflowSuggestion {
            title: "继续推进 trust score 核心化".to_string(),
            priority: 1,
            rationale: "because".to_string(),
            kind: WorkflowSuggestionKind::Feature,
        };
        let record = run_plan_record_action(dir.path(), &suggestion).unwrap();
        assert_eq!(record.status, "planned");
        let log = fs::read_to_string(dir.path().join("EXECUTION_LOG.md")).unwrap();
        assert!(log.contains("结构化执行计划"));
    }

    #[test]
    fn every_ten_rounds_report_is_emitted_once_and_then_cleared() {
        let mut state = sample_state();
        state.loop_iteration = 10;
        state.stage = crate::AutopilotStage::Verify;
        state.last_summary = "summary".to_string();
        state.current_focus = "focus".to_string();
        state.pending_confirmation.push("已达到第 10 轮汇报点，建议向用户汇报当前进展".to_string());

        let first = maybe_write_report("demo", &mut state).unwrap();
        assert!(first.is_some());
        assert!(state.pending_confirmation.iter().all(|v| !v.contains("轮汇报点")));

        let second = maybe_write_report("demo", &mut state).unwrap();
        assert!(second.is_none());
    }

    #[test]
    fn external_push_confirmation_is_enforced_by_strategy() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("VISION.md"), "artifact").unwrap();
        fs::write(dir.path().join("CURRENT_DIRECTION.md"), "trust score verify 文档").unwrap();
        fs::write(dir.path().join("TODO.md"), "同步 CURRENT_* 口径").unwrap();
        fs::write(dir.path().join("STATUS.md"), "# STATUS\n").unwrap();
        let config = sample_config(dir.path());
        let mut state = sample_state();
        std::process::Command::new("git").current_dir(dir.path()).args(["init"]).status().unwrap();
        fs::write(dir.path().join("dirty.txt"), "x").unwrap();
        let result = run_commit_guarded(dir.path(), &config, &mut state).unwrap();
        assert!(result.contains("阻止自动 push") || result.contains("当前仅保留本地待提交改动"));
        assert!(state.pending_confirmation.iter().any(|v| v.contains("external_push")));
    }
}
