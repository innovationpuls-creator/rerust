use rusqlite::{params, Connection};
use serde_json::json;
use uuid::Uuid;

use crate::llm::config::RuntimeConfig;
use crate::llm::server_proxy::{self, ServerLlmParams};

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn uuid() -> String {
    Uuid::new_v4().to_string()
}

fn clamp_score(value: f64) -> i32 {
    (value.round().max(0.0).min(100.0)) as i32
}

fn extract_json_object(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Some(v);
    }
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            let slice = &trimmed[start..=end];
            if let Ok(v) = serde_json::from_str(slice) {
                return Some(v);
            }
        }
    }
    None
}

fn extract_score_from_text(text: &str) -> Option<i32> {
    let patterns = [
        regex_lite::Regex::new(r"(?i)(?:Total Score|总分|综合评分)[^\d]{0,10}(\d{1,3})(?:\s*\/\s*100)?"),
        regex_lite::Regex::new(r"(\d{1,3})\s*/\s*100"),
        regex_lite::Regex::new(r"评分[^\d]{0,10}(\d{1,3})"),
    ];
    for re in &patterns {
        if let Ok(re) = re {
            if let Some(cap) = re.captures(text) {
                if let Some(score_str) = cap.get(1) {
                    if let Ok(n) = score_str.as_str().parse::<i32>() {
                        return Some(clamp_score(n as f64));
                    }
                }
            }
        }
    }
    None
}

fn extract_status_from_text(text: &str, score: Option<i32>) -> Option<&'static str> {
    let has_pass = regex_lite::Regex::new(r"(?i)(审核通过|通过|passed)").unwrap().is_match(text);
    let has_fail = regex_lite::Regex::new(r"(?i)(审核未通过|未通过|failed)").unwrap().is_match(text);
    if has_pass && !has_fail {
        return Some("passed");
    }
    if has_fail {
        return Some("failed");
    }
    if let Some(s) = score {
        return Some(if s >= 90 { "passed" } else { "failed" });
    }
    None
}

const SCRIPT_DIMENSIONS: &[(&str, f64)] = &[
    ("StoryProgress", 0.20),
    ("CharacterEmotion", 0.15),
    ("DialogueQuality", 0.15),
    ("PaceControl", 0.15),
    ("Readability", 0.15),
    ("AntiAI", 0.10),
    ("FormatCompliance", 0.10),
];

fn compute_weighted_score(dimensions: &[ReviewDimension]) -> i32 {
    let total: f64 = dimensions
        .iter()
        .enumerate()
        .map(|(i, d)| d.score as f64 * SCRIPT_DIMENSIONS[i].1)
        .sum();
    clamp_score(total)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReviewDimension {
    pub name: String,
    pub score: i32,
    pub comment: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReviewResult {
    pub score: i32,
    pub status: String,
    pub summary: String,
    pub issues: Vec<String>,
    pub suggestions: Vec<String>,
    pub dimensions: Vec<ReviewDimension>,
    pub priority: Vec<String>,
    pub rewrite_example: String,
    pub review_model: String,
    pub surgery_table: Option<Vec<serde_json::Value>>,
    pub revision_path: Option<Vec<String>>,
}

/// Build heuristic dimension scores from script text (local fallback).
fn build_local_dimension_scores(script_body: &str) -> Vec<ReviewDimension> {
    let text_len = script_body.trim().len();
    let scene_count = regex_lite::Regex::new(r"(?m)^※\s")
        .unwrap()
        .find_iter(script_body)
        .count();
    let dialogue_count = regex_lite::Regex::new(r"(?m)^[^\n：]{1,20}：")
        .unwrap()
        .find_iter(script_body)
        .count();
    let os_count = script_body.matches("OS").count();
    let hook_signals = ["重生", "背叛", "羞辱", "系统", "复仇", "反击", "打脸", "女帝", "阎罗", "危机"];
    let hook_hits = hook_signals.iter().filter(|k| script_body.contains(**k)).count();
    let conflict_signals = ["压制", "威胁", "羞辱", "挑衅", "逼迫", "翻盘", "反击", "怒斥"];
    let conflict_hits = conflict_signals.iter().filter(|k| script_body.contains(**k)).count();

    let s = |base: i32, inc: i32| clamp_score((base + inc) as f64);
    let story_progress = s(62, (scene_count as i32 * 5).min(18) + (conflict_hits as i32 * 3).min(10));
    let character = s(64, (dialogue_count as i32 * 2).min(16) + (os_count as i32 * 2).min(8));
    let dialogue = s(63, (dialogue_count as i32 * 2).min(18) + if script_body.contains('"') || script_body.contains('"') { 4 } else { 0 });
    let pace = s(64, (scene_count as i32 * 4).min(16) + if text_len > 1200 { 5 } else { 0 });
    let readability = s(64, (hook_hits as i32 * 3).min(16) + (conflict_hits as i32 * 3).min(10));
    let anti_ai = s(70, if text_len > 800 { 5 } else { 0 });
    let format = s(72, (scene_count as i32 * 3).min(10));

    vec![
        ReviewDimension { name: "StoryProgress".into(), score: story_progress, comment: "冲突推进有效，因果链完整。".into() },
        ReviewDimension { name: "CharacterEmotion".into(), score: character, comment: "角色行为一致，情感通过行为展示。".into() },
        ReviewDimension { name: "DialogueQuality".into(), score: dialogue, comment: "对白精炼有力，角色区分明显。".into() },
        ReviewDimension { name: "PaceControl".into(), score: pace, comment: "节奏精准，转场流畅。".into() },
        ReviewDimension { name: "Readability".into(), score: readability, comment: "钩子强力，爽点铺垫到位。".into() },
        ReviewDimension { name: "AntiAI".into(), score: anti_ai, comment: "文风自然，几乎无AI痕迹。".into() },
        ReviewDimension { name: "FormatCompliance".into(), score: format, comment: "格式规范，标记齐全。".into() },
    ]
}

fn build_local_review_result(script_body: &str, review_threshold: i32, default_model: &str) -> ReviewResult {
    let dimensions = build_local_dimension_scores(script_body);
    let score = compute_weighted_score(&dimensions);
    let status = if score >= review_threshold { "passed" } else { "failed" };

    let mut sorted = dimensions.clone();
    sorted.sort_by_key(|d| d.score);
    let low_dims: Vec<&ReviewDimension> = sorted.iter().take(3).collect();

    let issues: Vec<String> = low_dims.iter().map(|d| {
        match d.name.as_str() {
            "StoryProgress" => "中段推进和冲突抬升还不够集中，场景之间的递进感偏弱。",
            "CharacterEmotion" => "主角能动性与关键人物关系压制还不够明确，人物戏份还可以更抓人。",
            "DialogueQuality" => "部分对白还偏说明性，缺少更短更狠的短剧表达。",
            "PaceControl" => "节奏部分段落偏拖沓，可加快转场节奏。",
            "Readability" => "钩子和爽点铺垫还不够极致。",
            "AntiAI" => "部分文风有AI痕迹，需进一步润色。",
            _ => "高概念辨识度和商业钩子已经有基础，但还不够极致。",
        }.to_string()
    }).collect();

    let suggestions: Vec<String> = low_dims.iter().map(|d| {
        match d.name.as_str() {
            "StoryProgress" => "把中段改成更明显的层层升级，让每场戏都带来新的局势变化。",
            "CharacterEmotion" => "补强主角此刻最想达成的目标，并让关键配角明确承担压制或试探作用。",
            "DialogueQuality" => "把解释性台词压缩掉，改成更有攻击性、试探性或反击性的对白。",
            "PaceControl" => "缩短场景间的铺垫段落，加快关键冲突的到来。",
            "Readability" => "把第一场的羞辱、背叛或危险写得更具体，让开头商业识别度更强。",
            "AntiAI" => "增加具体细节和人物独特的表达方式，减少模板化表述。",
            _ => "强化高概念钩子与商业辨识度。",
        }.to_string()
    }).collect();

    let priority = vec!["先修结构节奏与关键节点强度。".to_string(), "再修主角目标与人物关系压制。".to_string(), "最后打磨对白锋利度和短剧化表达。".to_string()];
    let summary = if status == "passed" {
        "当前剧本已达到 CineForge 剧本审查通过线，可进入下游使用，但仍有局部可继续打磨。"
    } else {
        "当前剧本已有可用基础，但尚未达到 CineForge 90 分通过线，建议先按优先级回退优化。"
    };

    ReviewResult {
        score,
        status: status.to_string(),
        summary: summary.to_string(),
        issues,
        suggestions,
        dimensions,
        priority,
        rewrite_example: if status == "failed" { "优先把第一场写成正在发生的压迫戏，而不是概述背景，再让主角在同场里完成一次明确反应或回击。".to_string() } else { String::new() },
        review_model: format!("workfisher-review-master-v1 / local-heuristic / {}", default_model),
        surgery_table: None,
        revision_path: None,
    }
}

fn normalize_dimensions(input: Option<&[serde_json::Value]>, fallback: &[ReviewDimension]) -> Vec<ReviewDimension> {
    let input_dims = match input {
        Some(arr) => arr,
        None => return fallback.to_vec(),
    };
    SCRIPT_DIMENSIONS
        .iter()
        .enumerate()
        .map(|(i, (name, _weight))| {
            let matched = input_dims.iter().find(|v| {
                v.get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| n.trim().to_lowercase() == name.to_lowercase())
                    .unwrap_or(false)
            }).or_else(|| input_dims.get(i));
            let score = matched
                .and_then(|v| v.get("score").and_then(|s| s.as_i64()))
                .map(|s| clamp_score(s as f64))
                .unwrap_or(fallback.get(i).map(|d| d.score).unwrap_or(65));
            let comment = matched
                .and_then(|v| v.get("comment").and_then(|c| c.as_str()))
                .unwrap_or(fallback.get(i).map(|d| d.comment.as_str()).unwrap_or(""))
                .to_string();
            ReviewDimension {
                name: name.to_string(),
                score,
                comment,
            }
        })
        .collect()
}

fn normalize_review_payload(
    payload: &serde_json::Value,
    script_body: &str,
    review_threshold: i32,
    default_model: &str,
) -> ReviewResult {
    let fallback = build_local_review_result(script_body, review_threshold, default_model);
    let dimensions = normalize_dimensions(
        payload.get("dimensions").and_then(|v| v.as_array()).map(|v| v.as_slice()),
        &fallback.dimensions,
    );
    let weighted = compute_weighted_score(&dimensions);
    let score = payload.get("score").and_then(|s| s.as_i64()).map(|s| clamp_score(s as f64)).unwrap_or(weighted);
    let status = if score >= review_threshold { "passed" } else { "failed" };

    let surgery_table = payload.get("surgeryTable").and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter(|e| {
                e.get("original").and_then(|v| v.as_str()).unwrap_or("").len() > 0
                    || e.get("diagnosis").and_then(|v| v.as_str()).unwrap_or("").len() > 0
                    || e.get("rewrite").and_then(|v| v.as_str()).unwrap_or("").len() > 0
            })
            .cloned()
            .collect::<Vec<_>>()
    }).filter(|v: &Vec<_>| !v.is_empty());

    ReviewResult {
        score,
        status: status.to_string(),
        summary: payload
            .get("overallVerdict")
            .and_then(|v| v.as_str())
            .unwrap_or(&fallback.summary)
            .to_string(),
        issues: payload
            .get("problems")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .filter(|v: &Vec<String>| !v.is_empty())
            .unwrap_or(fallback.issues),
        suggestions: payload
            .get("suggestions")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .filter(|v: &Vec<String>| !v.is_empty())
            .unwrap_or(fallback.suggestions),
        dimensions,
        priority: payload
            .get("priority")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .filter(|v: &Vec<String>| !v.is_empty())
            .unwrap_or(fallback.priority),
        rewrite_example: payload
            .get("rewriteExample")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        review_model: format!("workfisher-review-master-v1 / {}", default_model),
        surgery_table,
        revision_path: None,
    }
}

async fn build_remote_review_result(
    _task_id: &str,
    script_body: &str,
    input_summary: &str,
    duration: &str,
    mode: &str,
    runtime_config: &RuntimeConfig,
) -> Result<ReviewResult, String> {
    let completion = server_proxy::request_server_llm(ServerLlmParams {
        prompt_slug: "script_review".into(),
        runtime_config: RuntimeConfig { ..runtime_config.clone() },
        temperature: Some(0.2),
        user_messages: vec![json!({
            "role": "user",
            "content": json!({
                "mode": mode,
                "inputSummary": input_summary,
                "duration": duration,
                "scriptBody": script_body,
                "reviewThreshold": runtime_config.review_threshold,
            }).to_string()
        })],
    })
    .await?;

    if let Some(json) = extract_json_object(&completion) {
        return Ok(normalize_review_payload(&json, script_body, runtime_config.review_threshold as i32, &runtime_config.default_model));
    }

    // Text-based extraction as fallback
    let score = extract_score_from_text(&completion)
        .unwrap_or_else(|| compute_weighted_score(&build_local_dimension_scores(script_body)));
    let status = extract_status_from_text(&completion, Some(score))
        .unwrap_or(if score >= runtime_config.review_threshold as i32 { "passed" } else { "failed" });

    let fallback = build_local_review_result(script_body, runtime_config.review_threshold as i32, &runtime_config.default_model);
    Ok(ReviewResult {
        score,
        status: status.to_string(),
        summary: fallback.summary,
        issues: fallback.issues,
        suggestions: fallback.suggestions,
        dimensions: fallback.dimensions,
        priority: fallback.priority,
        rewrite_example: fallback.rewrite_example,
        review_model: format!("workfisher-review-master-v1 / text-extraction / {}", runtime_config.default_model),
        surgery_table: None,
        revision_path: None,
    })
}

fn load_script_review_context(conn: &Connection, task_id: &str) -> Result<ScriptReviewContext, String> {
    let row = conn
        .query_row(
            "SELECT st.mode, st.input_summary, st.duration, so.script_body
             FROM script_tasks st
             LEFT JOIN script_outputs so ON so.id = (
               SELECT inner_so.id FROM script_outputs inner_so
               WHERE inner_so.task_id = st.id ORDER BY inner_so.created_at DESC LIMIT 1
             )
             WHERE st.id = ?1",
            params![task_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .map_err(|_| "未找到对应的剧本任务，无法执行审查。".to_string())?;

    let script_body = row.3.ok_or("当前任务还没有可审查的剧本文本，请先完成生成。")?;
    if script_body.trim().is_empty() {
        return Err("当前任务还没有可审查的剧本文本，请先完成生成。".into());
    }

    Ok(ScriptReviewContext {
        mode: row.0,
        input_summary: row.1.unwrap_or_default(),
        duration: row.2.unwrap_or_default(),
        script_body,
    })
}

struct ScriptReviewContext {
    mode: String,
    input_summary: String,
    duration: String,
    script_body: String,
}

fn persist_review_result(
    conn: &Connection,
    task_id: &str,
    result: &ReviewResult,
    review_time: &str,
) {
    conn.execute(
        "INSERT INTO review_records (id, task_id, score, status, summary, issues_json, suggestions_json, dimensions_json, priority_json, rewrite_example, surgery_table_json, revision_path_json, review_model, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            uuid(),
            task_id,
            result.score,
            result.status,
            result.summary,
            serde_json::to_string(&result.issues).unwrap_or_default(),
            serde_json::to_string(&result.suggestions).unwrap_or_default(),
            serde_json::to_string(&result.dimensions).unwrap_or_default(),
            serde_json::to_string(&result.priority).unwrap_or_default(),
            result.rewrite_example,
            result.surgery_table.as_ref().map(|v| serde_json::to_string(v).unwrap_or_default()),
            result.revision_path.as_ref().map(|v| serde_json::to_string(v).unwrap_or_default()),
            result.review_model,
            review_time,
        ],
    )
    .ok();

    let task_stage = if result.status == "passed" { "reviewed_passed" } else { "reviewed_failed" };
    conn.execute(
        "UPDATE script_tasks SET stage = ?1, updated_at = ?2 WHERE id = ?3",
        params![task_stage, review_time, task_id],
    )
    .ok();
}

/// Main entry: run script review (remote LLM or local heuristic).
pub async fn run_script_review(
    conn: &Connection,
    task_id: &str,
) -> Result<serde_json::Value, String> {
    let review_time = now();
    let settings = crate::db::crud::get_app_settings(conn);
    let runtime_config = RuntimeConfig {
        api_key: settings.text_key,
        api_base_url: settings.text_endpoint,
        default_model: settings.text_model,
        text_mode: settings.text_mode,
        mode: "remote-configured".into(),
        image_endpoint: String::new(),
        image_key: String::new(),
        image_model: String::new(),
        review_threshold: settings.review_threshold as u8,
        enable_local_save: settings.enable_local_save,
    };

    let ctx = load_script_review_context(conn, task_id)?;

    let result = build_remote_review_result(
        task_id,
        &ctx.script_body,
        &ctx.input_summary,
        &ctx.duration,
        &ctx.mode,
        &runtime_config,
    )
    .await?;

    persist_review_result(conn, task_id, &result, &review_time);

    Ok(json!({
        "taskId": task_id,
        "score": result.score,
        "status": result.status,
        "summary": result.summary,
        "issues": result.issues,
        "suggestions": result.suggestions,
        "dimensions": result.dimensions,
        "priority": result.priority,
        "rewriteExample": result.rewrite_example,
        "surgeryTable": result.surgery_table,
        "revisionPath": result.revision_path,
        "reviewedAt": review_time,
        "reviewModel": result.review_model,
    }))
}

/// Load persisted script review.
pub fn load_persisted_script_review(conn: &Connection, task_id: &str) -> Option<serde_json::Value> {
    let row = conn
        .query_row(
            "SELECT score, status, summary, issues_json, suggestions_json, dimensions_json, priority_json, rewrite_example, surgery_table_json, revision_path_json, review_model, created_at
             FROM review_records WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 1",
            params![task_id],
            |row| {
                Ok(json!({
                    "score": row.get::<_, Option<i32>>(0).ok().flatten(),
                    "status": row.get::<_, String>(1).ok(),
                    "summary": row.get::<_, Option<String>>(2).ok().flatten(),
                    "issues": row.get::<_, Option<String>>(3).ok().flatten().and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()),
                    "suggestions": row.get::<_, Option<String>>(4).ok().flatten().and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()),
                    "dimensions": row.get::<_, Option<String>>(5).ok().flatten().and_then(|s| serde_json::from_str::<Vec<ReviewDimension>>(&s).ok()),
                    "priority": row.get::<_, Option<String>>(6).ok().flatten().and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()),
                    "rewriteExample": row.get::<_, Option<String>>(7).ok().flatten(),
                    "surgeryTable": row.get::<_, Option<String>>(8).ok().flatten().and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(&s).ok()),
                    "revisionPath": row.get::<_, Option<String>>(9).ok().flatten().and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()),
                    "reviewModel": row.get::<_, Option<String>>(10).ok().flatten(),
                    "reviewedAt": row.get::<_, String>(11).ok(),
                }))
            },
        )
        .ok()?;
    Some(row)
}
