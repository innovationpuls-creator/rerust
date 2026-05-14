use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
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

// ── Data types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlineShot {
    pub index: usize,
    pub title: String,
    pub script_content: String,
    pub shot_type: String,
    pub key_beats: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outline {
    pub core_conflict: String,
    pub protagonist_motivation: String,
    pub information_gain: String,
    pub five_acts: Vec<String>,
    pub scene_quality: String,
    pub total_shots: usize,
    pub shots: Vec<OutlineShot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V2Shot {
    pub shot_number: usize,
    pub scene_index: usize,
    pub shot_type: String,
    pub title_bar: String,
    pub mount: String,
    pub camera: String,
    pub opening_frame: String,
    pub closing_frame: String,
    pub connection: String,
    pub transition: String,
    pub dual_anchor: String,
    pub main_prompt: String,
    pub compulsory_declaration: String,
    pub must_show: String,
    pub quality_route: String,
    pub imaging_style: String,
    pub quality_baseline: String,
    pub reference: String,
    pub micro_expressions: String,
    pub nail_lines: String,
    pub e15: String,
    pub asset_refs: Vec<String>,
}

const COMPULSORY_DECLARATION_DEFAULT: &str = "【强制声明】无背景音乐，仅保留环境音与人声：画面禁字幕/文字/水印/Logo：禁止可读文字：禁止超现实夸张：禁止无反作用力动作";
const QUALITY_BASELINE_DEFAULT: &str = "【画质底线】高光保结构、暗部不死黑、中间调厚实、主体边缘稳定、介质分层清楚";

// ── Asset helper ──

fn build_assets_json(conn: &Connection, task_id: &str) -> String {
    let assets = crate::db::crud::get_assets_by_task(conn, task_id);
    if assets.is_empty() {
        return "{}".to_string();
    }
    let mut characters = Vec::new();
    let mut scenes = Vec::new();
    let mut props = Vec::new();
    for a in &assets {
        let ty = a["assetType"].as_str().unwrap_or("");
        let data_str = a["assetDataJson"].as_str().unwrap_or("{}");
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(data_str) {
            match ty {
                "character" => characters.push(json!({
                    "name": data["name"], "role": data["role"], "aliases": data["aliases"],
                    "appearance": data["appearance"], "clothing": data["clothing"],
                    "personality": data["personality"], "colorPalette": data["colorPalette"],
                })),
                "scene" => scenes.push(json!({
                    "name": data["name"], "aliases": data["aliases"], "timeOfDay": data["timeOfDay"],
                    "atmosphere": data["atmosphere"], "materials": data["materials"],
                    "landmarks": data["landmarks"], "colorTemperature": data["colorTemperature"],
                })),
                "prop" => props.push(json!({
                    "name": data["name"], "aliases": data["aliases"],
                    "dramaticFunction": data["dramaticFunction"], "form": data["form"],
                    "material": data["material"], "surfaceState": data["surfaceState"],
                })),
                _ => {}
            }
        }
    }
    json!({ "characters": characters, "scenes": scenes, "props": props }).to_string()
}

fn safe_parse_assets(json: &str) -> serde_json::Value {
    serde_json::from_str(json).unwrap_or(json!({}))
}

// ── Load script context ──

fn load_script_context(conn: &Connection, task_id: &str) -> Result<(String, String), String> {
    let row: Result<(Option<String>, Option<String>), _> = conn.query_row(
        "SELECT so.script_body, st.duration
         FROM script_outputs so
         JOIN script_tasks st ON st.id = so.task_id
         WHERE so.task_id = ?1
         ORDER BY so.created_at DESC LIMIT 1",
        params![task_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    );
    match row {
        Ok((Some(body), duration)) if !body.trim().is_empty() => {
            Ok((body, duration.unwrap_or_else(|| "30秒".to_string())))
        }
        Ok(_) => Err("No script found for this task. Generate a script first.".into()),
        Err(_) => Err("No script found for this task. Generate a script first.".into()),
    }
}

// ── JSON extraction ──

fn extract_json(text: &str) -> Option<serde_json::Value> {
    let stripped = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim()
        .trim_end_matches("```")
        .trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(stripped) {
        return Some(v);
    }
    if let Some(start) = stripped.find('{') {
        if let Some(end) = stripped.rfind('}') {
            let slice = &stripped[start..=end];
            if let Ok(v) = serde_json::from_str(slice) {
                return Some(v);
            }
        }
    }
    None
}

// ── V2 normalize helpers ──

fn normalize_v2_shot(raw: &serde_json::Value, scene_index: usize) -> V2Shot {
    V2Shot {
        shot_number: scene_index + 1,
        scene_index,
        shot_type: raw.get("shotType").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        title_bar: raw.get("titleBar").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        mount: raw.get("mount").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        camera: raw.get("camera").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        opening_frame: raw.get("openingFrame").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        closing_frame: raw.get("closingFrame").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        connection: if scene_index == 0 {
            String::new()
        } else {
            raw.get("connection").and_then(|v| v.as_str()).unwrap_or("").to_string()
        },
        transition: raw.get("transition").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        dual_anchor: raw.get("dualAnchor").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        main_prompt: raw.get("mainPrompt").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        compulsory_declaration: raw
            .get("compulsoryDeclaration")
            .and_then(|v| v.as_str())
            .unwrap_or(COMPULSORY_DECLARATION_DEFAULT)
            .to_string(),
        must_show: raw.get("mustShow").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        quality_route: raw.get("qualityRoute").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        imaging_style: raw.get("imagingStyle").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        quality_baseline: raw
            .get("qualityBaseline")
            .and_then(|v| v.as_str())
            .unwrap_or(QUALITY_BASELINE_DEFAULT)
            .to_string(),
        reference: raw.get("reference").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        micro_expressions: raw.get("microExpressions").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        nail_lines: raw.get("nailLines").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        e15: raw.get("e15").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        asset_refs: raw
            .get("assetRefs")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
    }
}

fn build_fallback_v2_shot(scene_index: usize) -> V2Shot {
    V2Shot {
        shot_number: scene_index + 1,
        scene_index,
        shot_type: "D".to_string(),
        title_bar: format!(
            "【分镜{:02}｜15秒｜类型：D｜方案：T1建立压场模板｜档位：LITE｜节奏：建立→展开→定锚】",
            scene_index + 1
        ),
        mount: "挂载：@场景：待定".to_string(),
        camera: "相机位置：中景位；相机朝向：正面；角色朝向：面对镜头；构图锚点：主体居中".to_string(),
        opening_frame: "镜头平稳推入场景".to_string(),
        closing_frame: "主体定格在画面中央".to_string(),
        connection: String::new(),
        transition: String::new(),
        dual_anchor: "9:16 主体上下居中；16:9 左右留白平衡".to_string(),
        main_prompt: "REAL_CG，电影感写实质感。（请重新生成此镜）".to_string(),
        compulsory_declaration: COMPULSORY_DECLARATION_DEFAULT.to_string(),
        must_show: "待定".to_string(),
        quality_route: "自然光，高对比，主体清晰".to_string(),
        imaging_style: "稳定中景，不急不躁".to_string(),
        quality_baseline: QUALITY_BASELINE_DEFAULT.to_string(),
        reference: String::new(),
        micro_expressions: String::new(),
        nail_lines: String::new(),
        e15: String::new(),
        asset_refs: vec![],
    }
}

// ── Cross-shot context builder ──

fn build_previous_context(last_shot: &V2Shot, last_script_content: &str, scene_quality: &str) -> serde_json::Value {
    let len = last_script_content.len();
    let tail = if len > 200 {
        &last_script_content[len - 200..]
    } else {
        last_script_content
    };
    json!({
        "lastScriptTail": tail,
        "lastShotSummary": last_shot.title_bar,
        "lastClosingFrame": last_shot.closing_frame,
        "lastTransition": last_shot.transition,
        "sceneQuality": scene_quality,
    })
}

// ── Outline fallback ──

fn parse_outline_from_json(parsed: &serde_json::Value, _script_body: &str) -> Result<Outline, String> {
    let step03 = parsed
        .get("step03")
        .or_else(|| parsed.get("step0_3"))
        .or_else(|| parsed.get("step03Enhancement"))
        .or_else(|| parsed.get("analysis"))
        .unwrap_or(parsed);

    let core_conflict = step03
        .get("coreConflict")
        .or_else(|| parsed.get("coreConflict"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let protagonist_motivation = step03
        .get("protagonistMotivation")
        .or_else(|| step03.get("protagonistArc"))
        .or_else(|| step03.get("motivation"))
        .or_else(|| parsed.get("protagonistMotivation"))
        .or_else(|| parsed.get("protagonistArc"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let information_gain = step03
        .get("informationGain")
        .or_else(|| step03.get("infoReveal"))
        .or_else(|| step03.get("infoGain"))
        .or_else(|| step03.get("informationIncrement"))
        .or_else(|| parsed.get("informationGain"))
        .or_else(|| parsed.get("infoReveal"))
        .or_else(|| parsed.get("informationIncrement"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Five acts
    let five_acts_raw = parsed
        .get("fiveActs")
        .or_else(|| parsed.get("acts"))
        .or_else(|| parsed.get("eventChain"))
        .or_else(|| parsed.get("step00EventChain"))
        .or_else(|| parsed.get("step0").and_then(|v| v.get("acts")))
        .or_else(|| parsed.get("step0").and_then(|v| v.get("fiveActs")))
        .or_else(|| parsed.get("step00").and_then(|v| v.get("acts")))
        .or_else(|| parsed.get("step0"))
        .or_else(|| parsed.get("step00"));
    let five_acts: Vec<String> = five_acts_raw
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|a| {
                    if let Some(s) = a.as_str() {
                        s.to_string()
                    } else if let Some(obj) = a.as_object() {
                        obj.get("description")
                            .or_else(|| obj.get("content"))
                            .or_else(|| obj.get("text"))
                            .or_else(|| obj.get("act"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string()
                    } else {
                        a.to_string()
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // Shots array - try various key names
    let raw_shots = parsed
        .get("shots")
        .or_else(|| parsed.get("segments"))
        .or_else(|| parsed.get("shotList"))
        .or_else(|| parsed.get("step08ShotList"))
        .or_else(|| parsed.get("step08Shots"))
        .or_else(|| parsed.get("step08").and_then(|v| v.get("shots")))
        .or_else(|| parsed.get("step08").and_then(|v| v.get("segments")))
        .or_else(|| parsed.get("step08"));
    let raw_shots = match raw_shots.and_then(|v| v.as_array()) {
        Some(arr) if !arr.is_empty() => arr.clone(),
        _ => {
            // Fallback: scan all top-level keys for an array
            let mut found = None;
            if let Some(obj) = parsed.as_object() {
                for (_key, val) in obj {
                    if let Some(arr) = val.as_array() {
                        if !arr.is_empty() {
                            if let Some(first) = arr.first() {
                                if first.get("title").is_some()
                                    || first.get("summary").is_some()
                                    || first.get("scriptContent").is_some()
                                    || first.get("script").is_some()
                                    || first.get("content").is_some()
                                {
                                    found = Some(arr.clone());
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            found.unwrap_or_default()
        }
    };

    if raw_shots.is_empty() {
        return Err("大纲生成失败：未找到分镜列表".to_string());
    }

    let shots: Vec<OutlineShot> = raw_shots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let idx = s.get("index").and_then(|v| v.as_i64()).unwrap_or(i as i64) as usize;
            OutlineShot {
                index: idx,
                title: s
                    .get("title")
                    .or_else(|| s.get("summary"))
                    .or_else(|| s.get("description"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                script_content: s
                    .get("scriptContent")
                    .or_else(|| s.get("script"))
                    .or_else(|| s.get("excerpt"))
                    .or_else(|| s.get("content"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                shot_type: s
                    .get("shotType")
                    .or_else(|| s.get("type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("D")
                    .to_string(),
                key_beats: s
                    .get("keyBeats")
                    .or_else(|| s.get("beats"))
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default(),
            }
        })
        .collect();

    let scene_quality = parsed
        .get("sceneQuality")
        .or_else(|| parsed.get("quality"))
        .or_else(|| step03.get("sceneQuality"))
        .and_then(|v| v.as_str())
        .unwrap_or("电影感，古风写实，8k超高清")
        .to_string();

    Ok(Outline {
        core_conflict,
        protagonist_motivation,
        information_gain,
        five_acts,
        scene_quality,
        total_shots: shots.len(),
        shots,
    })
}

async fn run_outline_generation(
    conn: &Connection,
    task_id: &str,
    script_body: &str,
    duration: &str,
    runtime_config: &RuntimeConfig,
) -> Result<Outline, String> {
    let assets_json = build_assets_json(conn, task_id);
    let user_content = json!({
        "scriptBody": script_body,
        "duration": duration,
        "assets": safe_parse_assets(&assets_json),
    });
    let completion = server_proxy::request_server_llm(ServerLlmParams {
        runtime_config: RuntimeConfig { ..runtime_config.clone() },
        prompt_slug: "prompt_segment_planning".into(),
        temperature: Some(0.3),
        user_messages: vec![json!({"role": "user", "content": user_content.to_string()})],
    })
    .await?;

    let parsed = extract_json(&completion).ok_or_else(|| {
        let preview = if completion.len() > 200 {
            &completion[..200]
        } else {
            &completion
        };
        format!("大纲生成失败：无法解析 JSON。返回内容前200字：{}", preview)
    })?;

    parse_outline_from_json(&parsed, script_body)
}

// ── Outline public API ──

pub async fn generate_outline(
    conn: &Connection,
    input: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let now_time = now();
    let task_id = input["taskId"].as_str().unwrap_or("");
    if task_id.is_empty() {
        return Err("taskId is required".into());
    }

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

    let (script_body, duration) = load_script_context(conn, task_id)?;

    let outline = run_outline_generation(conn, task_id, &script_body, &duration, &runtime_config).await?;

    // Persist to DB
    let outline_json = serde_json::to_string(&outline).unwrap_or_default();
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM prompt_output_records WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 1",
            params![task_id],
            |row| row.get(0),
        )
        .ok();

    if let Some(oid) = existing {
        conn.execute(
            "UPDATE prompt_output_records SET grid_groups_json = ?1 WHERE id = ?2",
            params![&outline_json, &oid],
        )
        .ok();
    } else {
        conn.execute(
            "INSERT INTO prompt_output_records (id, task_id, grid_groups_json, seedance_groups_json, generation_model, created_at)
             VALUES (?1, ?2, ?3, '[]', 'outline-only', ?4)",
            params![uuid(), task_id, &outline_json, &now_time],
        )
        .ok();
    }

    Ok(json!({
        "taskId": task_id,
        "outline": outline,
        "generatedAt": now_time,
    }))
}

pub fn confirm_outline(conn: &Connection, input: &serde_json::Value) {
    let task_id = input["taskId"].as_str().unwrap_or("");
    if task_id.is_empty() {
        return;
    }
    let outline = input.get("outline");
    if outline.is_none() {
        return;
    }
    // Re-index shots
    let mut outline = outline.unwrap().clone();
    if let Some(shots) = outline.get_mut("shots").and_then(|v| v.as_array_mut()) {
        for (i, shot) in shots.iter_mut().enumerate() {
            shot["index"] = json!(i);
        }
        outline["totalShots"] = json!(shots.len());
    }

    let outline_json = outline.to_string();
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM prompt_output_records WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 1",
            params![task_id],
            |row| row.get(0),
        )
        .ok();

    if let Some(oid) = existing {
        conn.execute(
            "UPDATE prompt_output_records SET grid_groups_json = ?1 WHERE id = ?2",
            params![&outline_json, &oid],
        )
        .ok();
    } else {
        conn.execute(
            "INSERT INTO prompt_output_records (id, task_id, grid_groups_json, seedance_groups_json, generation_model, created_at)
             VALUES (?1, ?2, ?3, '[]', 'outline-confirmed', ?4)",
            params![uuid(), task_id, &outline_json, now()],
        )
        .ok();
    }
}

pub fn get_outline(conn: &Connection, task_id: &str) -> Option<serde_json::Value> {
    let row: Result<Option<String>, _> = conn.query_row(
        "SELECT grid_groups_json FROM prompt_output_records WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 1",
        params![task_id],
        |row| row.get(0),
    );
    match row {
        Ok(Some(json_str)) if !json_str.is_empty() && json_str != "[]" => {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_str) {
                if parsed.get("shots").and_then(|v| v.as_array()).is_some() {
                    return Some(parsed);
                }
            }
            None
        }
        _ => None,
    }
}

// ── Persistence ──

fn persist(
    conn: &Connection,
    task_id: &str,
    seedance_groups: &[V2Shot],
    model: &str,
    time: &str,
) {
    let seedance_json = serde_json::to_string(seedance_groups).unwrap_or_default();
    let existing_plan: Option<String> = conn
        .query_row(
            "SELECT grid_groups_json FROM prompt_output_records WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 1",
            params![task_id],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    let plan_json = existing_plan.unwrap_or_else(|| "[]".to_string());

    conn.execute("DELETE FROM prompt_output_records WHERE task_id = ?1", params![task_id])
        .ok();
    conn.execute(
        "INSERT INTO prompt_output_records (id, task_id, grid_groups_json, seedance_groups_json, generation_model, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![uuid(), task_id, &plan_json, &seedance_json, model, time],
    )
    .ok();
}

// ── Full prompt generation ──

pub async fn run_prompt_generation(
    conn: &Connection,
    input: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let now_time = now();
    let task_id = input["taskId"].as_str().unwrap_or("");
    if task_id.is_empty() {
        return Err("taskId is required".into());
    }

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

    let assets_json = build_assets_json(conn, task_id);
    let outline = get_outline(conn, task_id).ok_or_else(|| {
        "请先生成并确认分镜大纲，再开始逐镜生成。".to_string()
    })?;

    let shots = outline["shots"]
        .as_array()
        .ok_or_else(|| "大纲未包含分镜列表".to_string())?
        .clone();

    let total_shots = outline["totalShots"].as_i64().unwrap_or(shots.len() as i64) as usize;

    let mut all_shot_prompts: Vec<V2Shot> = Vec::new();
    let model = format!("workfisher-prompt-gen-v2 / {}", runtime_config.default_model);

    let mut previous_context: Option<serde_json::Value> = None;

    for (i, shot) in shots.iter().enumerate() {
        let prev = if i > 0 {
            shots.get(i - 1)
        } else {
            None
        };
        let next = shots.get(i + 1);

        let user_content = json!({
            "currentShot": {
                "scriptContent": shot.get("scriptContent").and_then(|v| v.as_str()).unwrap_or(""),
                "title": shot.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                "shotType": shot.get("shotType").and_then(|v| v.as_str()).unwrap_or("D"),
                "keyBeats": shot.get("keyBeats").or_else(|| shot.get("beats")),
                "index": i,
                "totalShots": total_shots,
            },
            "previousShotBrief": prev.map(|p| json!({
                "title": p.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                "shotType": p.get("shotType").and_then(|v| v.as_str()).unwrap_or(""),
                "keyBeats": p.get("keyBeats").or_else(|| p.get("beats")),
            })),
            "nextShotBrief": next.map(|n| json!({
                "title": n.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                "shotType": n.get("shotType").and_then(|v| v.as_str()).unwrap_or(""),
                "keyBeats": n.get("keyBeats").or_else(|| n.get("beats")),
            })),
            "storyOverview": {
                "coreConflict": outline.get("coreConflict").or_else(|| outline.get("coreConflict")),
                "protagonistMotivation": outline.get("protagonistMotivation"),
                "informationGain": outline.get("informationGain"),
                "fiveActs": outline.get("fiveActs").or_else(|| outline.get("fiveActs")),
                "sceneQuality": outline.get("sceneQuality"),
            },
            "assets": safe_parse_assets(&assets_json),
            "previousContext": previous_context,
        });

        let completion = server_proxy::request_server_llm(ServerLlmParams {
            runtime_config: RuntimeConfig { ..runtime_config.clone() },
            prompt_slug: "prompt_seedance_scene".into(),
            temperature: Some(0.3),
            user_messages: vec![json!({"role": "user", "content": user_content.to_string()})],
        })
        .await?;

        let parsed = extract_json(&completion);
        let group = match &parsed {
            Some(v) if v.get("mainPrompt").and_then(|p| p.as_str()).is_some() => {
                normalize_v2_shot(v, i)
            }
            _ => build_fallback_v2_shot(i),
        };

        all_shot_prompts.push(group.clone());
        previous_context = Some(build_previous_context(
            &group,
            shot.get("scriptContent").and_then(|v| v.as_str()).unwrap_or(""),
            outline.get("sceneQuality").and_then(|v| v.as_str()).unwrap_or(""),
        ));
    }

    persist(conn, task_id, &all_shot_prompts, &model, &now_time);

    Ok(json!({
        "taskId": task_id,
        "seedanceGroups": all_shot_prompts,
        "generatedAt": now_time,
        "generationModel": model,
    }))
}

// ── Single-shot (re)generation ──

pub async fn run_prompt_group_generation(
    conn: &Connection,
    input: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let now_time = now();
    let task_id = input["taskId"].as_str().unwrap_or("");
    if task_id.is_empty() {
        return Err("taskId is required".into());
    }
    let scene_index = input["sceneIndex"].as_i64().unwrap_or(0) as usize;

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

    let assets_json = build_assets_json(conn, task_id);
    let outline = get_outline(conn, task_id).ok_or_else(|| {
        "请先生成并确认分镜大纲。".to_string()
    })?;

    let shots = outline["shots"]
        .as_array()
        .ok_or_else(|| "大纲未包含分镜列表".to_string())?
        .clone();

    if scene_index >= shots.len() {
        return Err(format!(
            "分镜序号 {} 超出范围 (0-{})",
            scene_index,
            shots.len() - 1
        ));
    }

    let shot = &shots[scene_index];
    let total = outline["totalShots"].as_i64().unwrap_or(shots.len() as i64) as usize;

    // Build previousContext from existing data
    let mut previous_context: Option<serde_json::Value> = None;
    if scene_index > 0 {
        if let Ok(Some(json_str)) = conn.query_row::<Option<String>, _, _>(
            "SELECT seedance_groups_json FROM prompt_output_records WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 1",
            params![task_id],
            |row| row.get(0),
        ) {
                if let Ok(all_existing) =
                    serde_json::from_str::<Vec<serde_json::Value>>(&json_str)
                {
                    if let Some(prev_group) =
                        all_existing.iter().find(|g| g["sceneIndex"].as_i64() == Some(scene_index as i64 - 1))
                    {
                        let prev_shot = &shots[scene_index - 1];
                        previous_context = Some(build_previous_context(
                            &normalize_v2_shot(prev_group, scene_index - 1),
                            prev_shot["scriptContent"].as_str().unwrap_or(""),
                            outline["sceneQuality"].as_str().unwrap_or(""),
                        ));
                    }
                }
        }
    }

    let prev = if scene_index > 0 {
        Some(&shots[scene_index - 1])
    } else {
        None
    };
    let next = shots.get(scene_index + 1);

    let user_content = json!({
        "currentShot": {
            "scriptContent": shot.get("scriptContent").and_then(|v| v.as_str()).unwrap_or(""),
            "title": shot.get("title").and_then(|v| v.as_str()).unwrap_or(""),
            "shotType": shot.get("shotType").and_then(|v| v.as_str()).unwrap_or("D"),
            "keyBeats": shot.get("keyBeats").or_else(|| shot.get("beats")),
            "index": scene_index,
            "totalShots": total,
        },
        "previousShotBrief": prev.map(|p| json!({
            "title": p.get("title").and_then(|v| v.as_str()).unwrap_or(""),
            "shotType": p.get("shotType").and_then(|v| v.as_str()).unwrap_or(""),
            "keyBeats": p.get("keyBeats").or_else(|| p.get("beats")),
        })),
        "nextShotBrief": next.map(|n| json!({
            "title": n.get("title").and_then(|v| v.as_str()).unwrap_or(""),
            "shotType": n.get("shotType").and_then(|v| v.as_str()).unwrap_or(""),
            "keyBeats": n.get("keyBeats").or_else(|| n.get("beats")),
        })),
        "storyOverview": {
            "coreConflict": outline.get("coreConflict"),
            "protagonistMotivation": outline.get("protagonistMotivation"),
            "informationGain": outline.get("informationGain"),
            "fiveActs": outline.get("fiveActs"),
            "sceneQuality": outline.get("sceneQuality"),
        },
        "assets": safe_parse_assets(&assets_json),
        "previousContext": previous_context,
        "reviewFeedback": input.get("reviewFeedback"),
        "originalSeedance": input.get("originalSeedance"),
    });

    let completion = server_proxy::request_server_llm(ServerLlmParams {
        runtime_config: RuntimeConfig { ..runtime_config.clone() },
        prompt_slug: "prompt_seedance_scene".into(),
        temperature: Some(0.3),
        user_messages: vec![json!({"role": "user", "content": user_content.to_string()})],
    })
    .await?;

    let parsed = extract_json(&completion);
    let group = match &parsed {
        Some(v) if v.get("mainPrompt").and_then(|p| p.as_str()).is_some() => {
            normalize_v2_shot(v, scene_index)
        }
        _ => build_fallback_v2_shot(scene_index),
    };

    let model = format!("workfisher-prompt-gen-v2 / {}", runtime_config.default_model);

    // Merge into existing persisted data
    let existing_row: Option<String> = conn
        .query_row(
            "SELECT seedance_groups_json FROM prompt_output_records WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 1",
            params![task_id],
            |row| row.get(0),
        )
        .ok()
        .flatten();

    let mut all_seedance: Vec<V2Shot> = existing_row
        .and_then(|s| serde_json::from_str::<Vec<V2Shot>>(&s).ok())
        .unwrap_or_default();
    all_seedance.retain(|g| g.scene_index != scene_index);
    all_seedance.push(group.clone());
    all_seedance.sort_by_key(|g| g.scene_index);
    for (i, g) in all_seedance.iter_mut().enumerate() {
        g.shot_number = i + 1;
    }

    persist(conn, task_id, &all_seedance, &model, &now_time);

    Ok(json!({
        "taskId": task_id,
        "seedanceGroups": vec![group],
        "generatedAt": now_time,
        "generationModel": model,
    }))
}
