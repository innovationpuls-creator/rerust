use serde_json::Value;

// ── Template files (embedded at compile time) ──

const SELFCHECK_TEMPLATE: &str = include_str!("prompts/selfcheck.txt");
const CHECKPOINT_TEMPLATE: &str = include_str!("prompts/checkpoint.txt");

// ── PromptSlug template files ──

const SLUG_SCRIPT_PLANNING: &str = include_str!("prompts/slug_script_planning.txt");
const SLUG_SCRIPT_WRITING: &str = include_str!("prompts/slug_script_writing.txt");
const SLUG_SCRIPT_REVIEW: &str = include_str!("prompts/slug_script_review.txt");
const SLUG_ASSET_CHARACTER: &str = include_str!("prompts/slug_asset_character.txt");
const SLUG_ASSET_SCENE: &str = include_str!("prompts/slug_asset_scene.txt");
const SLUG_ASSET_PROP: &str = include_str!("prompts/slug_asset_prop.txt");
const SLUG_PROMPT_SEGMENT_PLANNING: &str = include_str!("prompts/slug_prompt_segment_planning.txt");
const SLUG_PROMPT_SEEDANCE_SCENE: &str = include_str!("prompts/slug_prompt_seedance_scene.txt");

// ── ContextType template files ──

const CTX_SEEDANCE_PHASE_AD: &str = include_str!("prompts/ctx_seedance_phase_ad.txt");
const CTX_SEEDANCE_UNIT_EFG: &str = include_str!("prompts/ctx_seedance_unit_efg.txt");

/// Build system prompt and user message for a given LLM context type.
pub fn build_prompt(context_type: &str, params: &Value) -> (String, String) {
    match context_type {
        "screenplay_step" => build_step_prompt(params),
        "screenplay_selfcheck" => build_selfcheck_prompt(params),
        "screenplay_checkpoint" => build_checkpoint_prompt(params),
        "seedance_phase_ad" => build_seedance_phase_ad_prompt(params),
        "seedance_unit_efg" => build_seedance_unit_efg_prompt(params),
        _ => (
            "你是一个专业的AI助手。".to_string(),
            format!(
                "请根据以下信息生成内容：\n{}",
                serde_json::to_string_pretty(params).unwrap_or_default()
            ),
        ),
    }
}

/// Build system prompt and user messages for a promptSlug-based LLM call.
/// Returns (system_prompt, user_messages_vec) where user_messages are
/// the original user messages from the caller.
pub fn build_prompt_slug(slug: &str, user_messages: &[Value]) -> (String, Vec<Value>) {
    let system = match slug {
        "script_planning" => SLUG_SCRIPT_PLANNING,
        "script_writing" => SLUG_SCRIPT_WRITING,
        "script_review" => SLUG_SCRIPT_REVIEW,
        "asset_character" => SLUG_ASSET_CHARACTER,
        "asset_scene" => SLUG_ASSET_SCENE,
        "asset_prop" => SLUG_ASSET_PROP,
        "prompt_segment_planning" => SLUG_PROMPT_SEGMENT_PLANNING,
        "prompt_seedance_scene" => SLUG_PROMPT_SEEDANCE_SCENE,
        _ => "你是一个专业的AI助手。",
    };
    (system.to_string(), user_messages.to_vec())
}

// ── Step prompt builder ──

fn build_step_prompt(params: &Value) -> (String, String) {
    let step_number = params["stepNumber"].as_u64().unwrap_or(1);
    let init = &params["init"];
    let project_snapshot = &params["projectSnapshot"];
    let user_feedback = params["userFeedback"].as_str().unwrap_or("");

    let duration = init["duration"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("2分钟");
    let seconds = parse_seconds(duration);

    let mut system = build_step_system(step_number, duration, &seconds);

    // Prep config block at the top so step templates' "顶部" references find it
    let mut config_block = String::new();
    if let Some(f) = init["format"].as_str().filter(|s| !s.is_empty()) {
        config_block.push_str(&format!("format: {}\n", f));
    }
    if let Some(u) = init["ultrashortMode"].as_str().filter(|s| !s.is_empty()) {
        config_block.push_str(&format!("ultrashortMode: {}\n", u));
    }
    if let Some(g) = init["genres"].as_array() {
        let genres: Vec<&str> = g.iter().filter_map(|v| v.as_str()).collect();
        if !genres.is_empty() {
            config_block.push_str(&format!("genres: {}\n", genres.join("、")));
        }
    }
    if init["chinese"].as_bool().unwrap_or(false) {
        config_block.push_str("chinese: true\n");
    }
    if !config_block.is_empty() {
        system = format!("## 项目配置\n{}\n\n{}", config_block, system);
    }

    let user = build_user_message(step_number, init, project_snapshot, user_feedback);

    (system, user)
}

fn build_step_system(step_number: u64, duration: &str, seconds: &str) -> String {
    let template = match step_number {
        1 => include_str!("prompts/step1.txt"),
        2 => include_str!("prompts/step2.txt"),
        3 => include_str!("prompts/step3.txt"),
        4 => include_str!("prompts/step4.txt"),
        5 => include_str!("prompts/step5.txt"),
        6 => include_str!("prompts/step6.txt"),
        7 => include_str!("prompts/step7.txt"),
        8 => include_str!("prompts/step8.txt"),
        _ => return String::new(),
    };

    template
        .replace("{duration}", duration)
        .replace("{seconds}", seconds)
}

fn parse_seconds(duration: &str) -> String {
    let cleaned: String = duration.chars().filter(|c| c.is_ascii_digit()).collect();
    if let Ok(n) = cleaned.parse::<u32>() {
        (n * 60).to_string()
    } else {
        "120".to_string()
    }
}

// ── User message builder ──

fn build_user_message(
    step_number: u64,
    init: &Value,
    project_snapshot: &Value,
    user_feedback: &str,
) -> String {
    let name = init["name"].as_str().filter(|s| !s.is_empty());
    let concept = init["concept"].as_str().filter(|s| !s.is_empty());

    let mut msg = String::new();

    // Include project concept (the user's original idea)
    if let Some(c) = concept {
        msg.push_str(&format!("## 项目核心概念\n{}\n\n", c));
    }
    if let Some(n) = name {
        if name != concept {
            msg.push_str(&format!("## 项目名称\n{}\n\n", n));
        }
    }

    // Include project config fields for template branching
    if let Some(f) = init["format"].as_str().filter(|s| !s.is_empty()) {
        msg.push_str(&format!("## 项目格式\n{}\n\n", f));
    }
    if let Some(u) = init["ultrashortMode"].as_str().filter(|s| !s.is_empty()) {
        msg.push_str(&format!("## 超短片模式\n{}\n\n", u));
    }
    if let Some(g) = init["genres"].as_array() {
        let genres: Vec<&str> = g.iter().filter_map(|v| v.as_str()).collect();
        if !genres.is_empty() {
            msg.push_str(&format!("## 题材\n{}\n\n", genres.join("、")));
        }
    }
    if init["chinese"].as_bool().unwrap_or(false) {
        msg.push_str("## 中式叙事\n启用\n\n");
    }

    if step_number == 6 || step_number == 7 {
        let duration = init["duration"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("2分钟");
        msg.push_str(&format!(
            "\n## ⚠ 时长红线\n\
             本步涉及具体场景时长。所有场景 duration 加起来必须 ≈ **{}** (±10%)。\n\
             输出前先自己把 duration 字段转秒累加一次，验证在目标区间内再输出。\n\n",
            duration
        ));
    }

    // Include previous steps' structured output as context
    if let Some(snapshot) = project_snapshot.as_object() {
        if let Some(steps) = snapshot.get("steps").and_then(|s| s.as_object()) {
            for n in 1..step_number {
                let key = n.to_string();
                if let Some(step_data) = steps.get(&key) {
                    if let Some(structured) = step_data.get("structured") {
                        if !structured.is_null() {
                            let pretty =
                                serde_json::to_string_pretty(structured).unwrap_or_default();
                            let truncated: String = pretty.chars().take(1500).collect();
                            msg.push_str(&format!("### 第{}步产出\n{}\n\n", n, truncated));
                        }
                    }
                }
            }
        }
    }

    if !user_feedback.is_empty() {
        msg.push_str(&format!("## 用户修改要求\n{}\n\n", user_feedback));
    }

    msg.push_str(
        "请严格按 system 中 \"当前任务\" 的输出格式要求产出，不要输出任何 meta 解释。",
    );

    msg
}

// ── Selfcheck prompt builder ──

fn build_selfcheck_prompt(params: &Value) -> (String, String) {
    let step_number = params["stepNumber"].as_u64().unwrap_or(1);
    let current_output = params["currentOutput"].as_str().unwrap_or("(无产出)");
    let init = &params["init"];

    let concept = init["concept"].as_str().unwrap_or("");
    let duration = init["duration"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    let _name = init["name"].as_str().unwrap_or("");

    let system = SELFCHECK_TEMPLATE.replace("{stepNumber}", &step_number.to_string());

    let user = format!(
        "## 待自检 · 第 {} 步 (当前产出)\n\n\
         项目概念：{}\n时长：{}\n\n\
         ```\n{}\n```\n\n\
         请严格按 system 中 \"输出格式\" 给出 markdown 自检报告 \
         (## CHECK N 块 · === 分隔)。不要输出任何其他文字。",
        step_number, concept, duration, current_output
    );

    (system, user)
}

// ── Checkpoint prompt builder ──

fn build_checkpoint_prompt(params: &Value) -> (String, String) {
    let init = &params["init"];
    let _project_snapshot = &params["projectSnapshot"];

    let name = init["name"].as_str().unwrap_or("");
    let concept = init["concept"].as_str().unwrap_or("");
    let duration = init["duration"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    let format_val = init["format"]
        .as_str()
        .unwrap_or("");
    let project_snapshot_str = serde_json::to_string_pretty(&params["projectSnapshot"])
        .unwrap_or_default();

    let system = CHECKPOINT_TEMPLATE.to_string();

    let user = format!(
        "## 项目信息\n\
         - 项目名: {}\n\
         - 格式: {}\n\
         - 时长: {}\n\
         - 概念: {}\n\n\
         ## 前置步骤产出 (请据此生成检查点)\n\n\
         {}\n\n\
         请按 system 中的模板产出检查点。字段完整，简洁精准，800-1200 字。",
        name, format_val, duration, concept, project_snapshot_str
    );

    (system, user)
}

// ── Seedance prompt builders ──

fn build_seedance_phase_ad_prompt(params: &Value) -> (String, String) {
    let system = CTX_SEEDANCE_PHASE_AD.to_string();
    let script_body = params["scriptBody"].as_str().unwrap_or("");
    let duration = params["duration"].as_str().unwrap_or("");
    let assets_json = params["assetsJson"].as_str().unwrap_or("{}");

    let user = format!(
        "## 剧本正文\n\n{}\n\n## 时长\n{}\n\n## 资产清单\n\n{}",
        script_body, duration, assets_json
    );

    (system, user)
}

fn build_seedance_unit_efg_prompt(params: &Value) -> (String, String) {
    let system = CTX_SEEDANCE_UNIT_EFG.to_string();

    let unit = serde_json::to_string_pretty(&params["unit"]).unwrap_or_default();
    let script_fragment = params["scriptFragment"].as_str().unwrap_or("");
    let analysis_context = serde_json::to_string_pretty(&params["analysisContext"]).unwrap_or_default();
    let assets_json = params["assetsJson"].as_str().unwrap_or("{}");
    let unit_index = params["unitIndex"].as_i64().unwrap_or(0);
    let planned_entry_state = params["plannedEntryState"].as_str().unwrap_or("");
    let previous_plan_exit = params["previousPlanExit"].as_str().unwrap_or("");

    let user = format!(
        "## 单元信息\n\n{}\n\n## 剧本片段\n\n{}\n\n## 分析上下文\n\n{}\n\n## 资产清单\n\n{}\n\n## 单元序号\n{}\n\n## 起幅锚点\n{}\n\n## 上一单元落幅\n{}",
        unit, script_fragment, analysis_context, assets_json, unit_index, planned_entry_state, previous_plan_exit
    );

    (system, user)
}
