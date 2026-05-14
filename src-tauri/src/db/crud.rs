use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn uuid() -> String {
    Uuid::new_v4().to_string()
}

// ── App Settings ──

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub text_endpoint: String,
    pub text_key: String,
    pub text_model: String,
    pub text_mode: String,
    pub image_endpoint: String,
    pub image_key: String,
    pub image_model: String,
    pub review_threshold: i32,
    pub enable_local_save: bool,
}

pub fn get_app_settings(conn: &Connection) -> AppSettings {
    let mut stmt = conn
        .prepare("SELECT setting_key, setting_value FROM app_settings")
        .unwrap();
    let rows: Vec<(String, Option<String>)> = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get(1)?)))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    let mut map = std::collections::HashMap::new();
    for (k, v) in &rows {
        if let Some(val) = v {
            map.insert(k.as_str(), val.as_str());
        }
    }

    AppSettings {
        text_endpoint: map.get("textEndpoint").unwrap_or(&"").to_string(),
        text_key: map.get("textKey").unwrap_or(&"").to_string(),
        text_model: map
            .get("textModel")
            .unwrap_or(&"deepseek-reasoner")
            .to_string(),
        text_mode: {
            let mode = map.get("textMode").unwrap_or(&"openai");
            matches!(*mode, "openai" | "gemini" | "anthropic")
                .then(|| mode.to_string())
                .unwrap_or_else(|| "openai".to_string())
        },
        image_endpoint: map.get("imageEndpoint").unwrap_or(&"").to_string(),
        image_key: map.get("imageKey").unwrap_or(&"").to_string(),
        image_model: map.get("imageModel").unwrap_or(&"").to_string(),
        review_threshold: map
            .get("reviewThreshold")
            .and_then(|s| s.parse::<i32>().ok())
            .map(|v| v.clamp(0, 100))
            .unwrap_or(90),
        enable_local_save: map
            .get("enableLocalSave")
            .map(|s| s == &"1")
            .unwrap_or(true),
    }
}

pub fn save_app_settings(conn: &Connection, input: &AppSettings) -> AppSettings {
    let t = now();
    let mut stmt = conn
        .prepare_cached(
            "INSERT INTO app_settings (id, setting_key, setting_value, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(setting_key) DO UPDATE SET
               setting_value = excluded.setting_value,
               updated_at = excluded.updated_at",
        )
        .unwrap();

    let pairs: [(&str, &str); 9] = [
        ("textEndpoint", &input.text_endpoint),
        ("textKey", &input.text_key),
        ("textModel", &input.text_model),
        ("textMode", &input.text_mode),
        ("imageEndpoint", &input.image_endpoint),
        ("imageKey", &input.image_key),
        ("imageModel", &input.image_model),
        (
            "reviewThreshold",
            &input.review_threshold.to_string(),
        ),
        (
            "enableLocalSave",
            if input.enable_local_save { "1" } else { "0" },
        ),
    ];

    for (key, val) in &pairs {
        stmt.execute(params![key, key, val, &t]).ok();
    }

    get_app_settings(conn)
}

// ── Database Meta ──

#[derive(Debug, Serialize)]
pub struct DatabaseMeta {
    pub db_path: String,
    pub data_dir: String,
}

// ── Script Drafts ──

#[derive(Debug, Deserialize)]
pub struct ScriptDraftInput {
    pub mode: String,
    pub input_summary: String,
    pub genre: Option<String>,
    pub style: Option<String>,
    pub duration: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ScriptDraftResult {
    pub project_id: String,
    pub task_id: String,
    pub project_name: String,
    pub saved_at: String,
}

fn script_project_name(mode: &str, input_summary: &str) -> String {
    let prefix = match mode {
        "plot" => "剧情描述生成剧本",
        "image" => "图片生成连续性剧本",
        "rewrite" => "剧本优化重生系统",
        _ => "剧本",
    };
    let s = input_summary.trim();
    if s.is_empty() {
        return format!("{}草稿", prefix);
    }
    let truncated: String = s.chars().take(18).collect();
    format!("{} - {}", prefix, truncated)
}

pub fn save_script_draft(conn: &Connection, input: &ScriptDraftInput) -> ScriptDraftResult {
    let project_id = uuid();
    let task_id = uuid();
    let t = now();
    let project_name = script_project_name(&input.mode, &input.input_summary);

    conn.execute(
        "INSERT INTO projects (id, name, module_type, status, created_at, updated_at)
         VALUES (?1, ?2, 'script', 'draft', ?3, ?4)",
        params![project_id, project_name, t, t],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO script_tasks (id, project_id, mode, input_summary, genre, style, duration, stage, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'idle', ?8, ?9)",
        params![
            task_id,
            project_id,
            input.mode,
            input.input_summary.trim(),
            input.genre.as_deref().unwrap_or_default(),
            input.style.as_deref().unwrap_or_default(),
            input.duration.as_deref().unwrap_or_default(),
            t,
            t,
        ],
    )
    .unwrap();

    ScriptDraftResult {
        project_id,
        task_id,
        project_name,
        saved_at: t,
    }
}

// ── Image / Video Prompt Drafts ──

#[derive(Debug, Deserialize)]
pub struct ImageVideoDraftInput {
    pub mode: String,
    pub source_script: Option<String>,
    pub visual_style: Option<String>,
    pub image_goal: Option<String>,
    pub script_beats: Option<String>,
    pub video_style: Option<String>,
    pub motion_focus: Option<String>,
    pub project_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DraftResult {
    pub project_id: String,
    pub task_id: String,
    pub saved_at: String,
}

pub fn save_image_draft(conn: &Connection, input: &ImageVideoDraftInput) -> DraftResult {
    let project_id = input.project_id.clone().unwrap_or_else(uuid);
    let task_id = uuid();
    let t = now();
    conn.execute(
        "INSERT INTO projects (id, name, module_type, status, created_at, updated_at)
         VALUES (?1, '图片提示词草稿', 'image', 'draft', ?2, ?3)
         ON CONFLICT(id) DO NOTHING",
        params![project_id, t, t],
    )
    .ok();
    conn.execute(
        "INSERT INTO image_tasks (id, project_id, mode, source_script, visual_style, image_goal, stage, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'idle', ?7, ?8)",
        params![
            task_id,
            project_id,
            input.mode,
            input.source_script.as_deref().unwrap_or_default(),
            input.visual_style.as_deref().unwrap_or_default(),
            input.image_goal.as_deref().unwrap_or_default(),
            t,
            t,
        ],
    )
    .unwrap();
    DraftResult { project_id, task_id, saved_at: t }
}

pub fn save_video_draft(conn: &Connection, input: &ImageVideoDraftInput) -> DraftResult {
    let project_id = input.project_id.clone().unwrap_or_else(uuid);
    let task_id = uuid();
    let t = now();
    conn.execute(
        "INSERT INTO projects (id, name, module_type, status, created_at, updated_at)
         VALUES (?1, '视频提示词草稿', 'video', 'draft', ?2, ?3)
         ON CONFLICT(id) DO NOTHING",
        params![project_id, t, t],
    )
    .ok();
    conn.execute(
        "INSERT INTO video_tasks (id, project_id, mode, script_beats, video_style, motion_focus, stage, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'idle', ?7, ?8)",
        params![
            task_id,
            project_id,
            input.mode,
            input.script_beats.as_deref().unwrap_or_default(),
            input.video_style.as_deref().unwrap_or_default(),
            input.motion_focus.as_deref().unwrap_or_default(),
            t,
            t,
        ],
    )
    .unwrap();
    DraftResult { project_id, task_id, saved_at: t }
}

// ── Script Generation ──

#[derive(Debug, Deserialize)]
pub struct ScriptGenerationInput {
    pub mode: String,
    pub duration: Option<String>,
    pub input_summary: String,
    pub style_preset: Option<String>,
    pub genres: Option<String>,
    pub audience: Option<String>,
    pub tone: Option<String>,
    pub ending: Option<String>,
    pub output_mode: Option<String>,
    pub episodes: Option<String>,
    pub custom_style: Option<String>,
    pub existing_project_id: Option<String>,
    pub existing_task_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ScriptSection {
    pub title: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct ScriptGenerationResult {
    pub project_id: String,
    pub task_id: String,
    pub stage: String,
    pub generated_at: String,
    pub project_name: String,
    pub sections: Vec<ScriptSection>,
    pub characters: Vec<serde_json::Value>,
}

pub fn fallback_sections(mode: &str) -> Vec<ScriptSection> {
    let text = match mode {
        "image" => "【图片连续性剧本预演】\n系统正在从参考图片中反推人物身份、视觉线索与冲突起点。",
        "rewrite" => "【剧本优化重生系统预演】\n系统将保留原剧情主线，优先压缩冗余、强化冲突、提前爆点。",
        _ => "【剧情描述生成剧本预演】\n系统会围绕重生、背叛、反击三条线组织故事。",
    };
    vec![ScriptSection { title: "完整结果文本".into(), content: text.into() }]
}

pub fn save_script_generation(
    conn: &Connection,
    input: &ScriptGenerationInput,
    sections: Vec<ScriptSection>,
    characters: Vec<serde_json::Value>,
    raw_response: Option<serde_json::Value>,
) -> ScriptGenerationResult {
    let t = now();
    let project_id = input.existing_project_id.clone().unwrap_or_else(uuid);
    let task_id = input.existing_task_id.clone().unwrap_or_else(uuid);
    let project_name = script_project_name(&input.mode, &input.input_summary);

    conn.execute(
        "INSERT INTO projects (id, name, module_type, status, created_at, updated_at)
         VALUES (?1, ?2, 'script', 'active', ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, status = excluded.status, updated_at = excluded.updated_at",
        params![project_id, project_name, t, t],
    ).unwrap();
    conn.execute(
        "INSERT INTO script_tasks (id, project_id, mode, input_summary, genre, style, duration, stage, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, '', '', ?5, 'ready', ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET mode = excluded.mode, input_summary = excluded.input_summary, stage = excluded.stage, updated_at = excluded.updated_at",
        params![task_id, project_id, input.mode, input.input_summary.trim(), input.duration.as_deref().unwrap_or_default(), t, t],
    ).unwrap();

    conn.execute("DELETE FROM script_outputs WHERE task_id = ?1", params![task_id]).ok();

    let script_body: String = sections.iter().map(|s| s.content.as_str()).collect::<Vec<_>>().join("\n\n");
    let chars_json = serde_json::to_string(&characters).unwrap_or_default();
    let raw = raw_response.map(|v| v.to_string()).unwrap_or_default();

    conn.execute(
        "INSERT INTO script_outputs (id, task_id, characters_json, plot_outline, script_body, hook_opening, storyboard_base, raw_response, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![uuid(), task_id, chars_json, script_body, script_body, script_body, script_body, raw, t],
    ).unwrap();

    ScriptGenerationResult { project_id, task_id, stage: "ready".into(), generated_at: t, project_name, sections, characters }
}

pub fn update_script_body(conn: &Connection, task_id: &str, new_body: &str) {
    let output_id: Option<String> = conn
        .query_row(
            "SELECT id FROM script_outputs WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 1",
            params![task_id],
            |row| row.get(0),
        )
        .ok();
    if let Some(ref oid) = output_id {
        conn.execute("UPDATE script_outputs SET script_body = ?1, plot_outline = ?1 WHERE id = ?2", params![new_body, oid])
            .ok();
    }
    conn.execute("UPDATE script_tasks SET updated_at = ?1 WHERE id = ?2", params![now(), task_id])
        .ok();
}

// ── Import Existing Script ──

#[derive(Debug, Deserialize)]
pub struct ImportScriptInput {
    pub script_body: String,
    pub input_summary: Option<String>,
    pub duration: Option<String>,
}

pub fn import_existing_script(conn: &Connection, input: &ImportScriptInput) -> ScriptGenerationResult {
    let t = now();
    let project_id = uuid();
    let task_id = uuid();
    let clean_body = input.script_body.trim().to_string();
    let first_line = clean_body.lines().find(|l| !l.trim().is_empty()).unwrap_or("导入剧本").trim().to_string();
    let truncated: String = first_line.chars().take(24).collect();
    let project_name = format!("导入剧本 - {}", truncated);
    let summary: String = input.input_summary.clone().unwrap_or(first_line).chars().take(200).collect();

    conn.execute(
        "INSERT INTO projects (id, name, module_type, status, created_at, updated_at) VALUES (?1, ?2, 'script', 'active', ?3, ?4)",
        params![project_id, project_name, t, t],
    ).unwrap();
    conn.execute(
        "INSERT INTO script_tasks (id, project_id, mode, input_summary, genre, style, duration, stage, created_at, updated_at)
         VALUES (?1, ?2, 'plot', ?3, '', '', ?4, 'ready', ?5, ?6)",
        params![task_id, project_id, summary, input.duration.as_deref().unwrap_or("2分钟"), t, t],
    ).unwrap();

    let raw = serde_json::json!({ "sections": [{ "title": "导入的剧本内容", "content": clean_body }] });
    conn.execute(
        "INSERT INTO script_outputs (id, task_id, characters_json, plot_outline, script_body, hook_opening, storyboard_base, raw_response, created_at)
         VALUES (?1, ?2, '[]', ?3, ?4, ?5, ?6, ?7, ?8)",
        params![uuid(), task_id, clean_body, clean_body, clean_body, clean_body, raw.to_string(), t],
    ).unwrap();

    ScriptGenerationResult { project_id, task_id, stage: "ready".into(), generated_at: t, project_name, sections: vec![ScriptSection { title: "导入的剧本内容".into(), content: clean_body }], characters: vec![] }
}

// ── Script Tasks History ──

#[derive(Debug, Serialize)]
pub struct ScriptTaskSummary {
    pub task_id: String,
    pub project_id: String,
    pub project_name: String,
    pub mode: String,
    pub input_summary: String,
    pub genre: String,
    pub style: String,
    pub duration: String,
    pub stage: String,
    pub updated_at: String,
    pub review_score: Option<i32>,
    pub review_status: Option<String>,
}

pub fn get_recent_script_tasks(conn: &Connection, limit: i64) -> Vec<ScriptTaskSummary> {
    let mut stmt = conn.prepare(
        "SELECT st.id, st.project_id, p.name, st.mode, st.input_summary, st.genre, st.style, st.duration, st.stage, st.updated_at, rr.score, rr.status
         FROM script_tasks st INNER JOIN projects p ON p.id = st.project_id
         LEFT JOIN review_records rr ON rr.id = (SELECT inner_rr.id FROM review_records inner_rr WHERE inner_rr.task_id = st.id ORDER BY inner_rr.created_at DESC LIMIT 1)
         ORDER BY st.updated_at DESC LIMIT ?1",
    ).unwrap();
    stmt.query_map(params![limit], |row| {
        Ok(ScriptTaskSummary {
            task_id: row.get(0)?, project_id: row.get(1)?, project_name: row.get(2)?, mode: row.get(3)?,
            input_summary: row.get::<_, Option<String>>(4)?.unwrap_or_default(), genre: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
            style: row.get::<_, Option<String>>(6)?.unwrap_or_default(), duration: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
            stage: row.get(8)?, updated_at: row.get(9)?, review_score: row.get(10)?, review_status: row.get(11)?,
        })
    }).unwrap().filter_map(|r| r.ok()).collect()
}

pub fn get_recent_image_tasks(conn: &Connection, limit: i64) -> Vec<serde_json::Value> {
    let mut stmt = conn.prepare("SELECT id, project_id, mode, stage, updated_at FROM image_tasks ORDER BY updated_at DESC LIMIT ?1").unwrap();
    stmt.query_map(params![limit], |row| {
        Ok(serde_json::json!({"taskId": row.get::<_, String>(0)?, "projectId": row.get::<_, String>(1)?, "mode": row.get::<_, String>(2)?, "stage": row.get::<_, String>(3)?, "updatedAt": row.get::<_, String>(4)?}))
    }).unwrap().filter_map(|r| r.ok()).collect()
}

pub fn get_recent_video_tasks(conn: &Connection, limit: i64) -> Vec<serde_json::Value> {
    let mut stmt = conn.prepare("SELECT id, project_id, mode, stage, updated_at FROM video_tasks ORDER BY updated_at DESC LIMIT ?1").unwrap();
    stmt.query_map(params![limit], |row| {
        Ok(serde_json::json!({"taskId": row.get::<_, String>(0)?, "projectId": row.get::<_, String>(1)?, "mode": row.get::<_, String>(2)?, "stage": row.get::<_, String>(3)?, "updatedAt": row.get::<_, String>(4)?}))
    }).unwrap().filter_map(|r| r.ok()).collect()
}

// ── Load Script Task ──

pub fn load_script_task(conn: &Connection, task_id: &str) -> Option<serde_json::Value> {
    let task = conn.query_row(
        "SELECT st.*, p.name as project_name, p.status as project_status FROM script_tasks st JOIN projects p ON p.id = st.project_id WHERE st.id = ?1",
        params![task_id],
        |row| Ok(serde_json::json!({
            "taskId": row.get::<_, String>("id").ok(), "projectId": row.get::<_, String>("project_id").ok(),
            "projectName": row.get::<_, String>("project_name").ok(), "mode": row.get::<_, String>("mode").ok(),
            "inputSummary": row.get::<_, Option<String>>("input_summary").ok().flatten(),
            "genre": row.get::<_, Option<String>>("genre").ok().flatten(), "style": row.get::<_, Option<String>>("style").ok().flatten(),
            "duration": row.get::<_, Option<String>>("duration").ok().flatten(), "stage": row.get::<_, String>("stage").ok(),
            "status": row.get::<_, String>("project_status").ok(), "createdAt": row.get::<_, String>("created_at").ok(),
            "updatedAt": row.get::<_, String>("updated_at").ok(),
        })),
    ).ok()?;

    let outputs: Vec<serde_json::Value> = {
        let mut stmt = conn.prepare("SELECT * FROM script_outputs WHERE task_id = ?1 ORDER BY created_at DESC").unwrap();
        stmt.query_map(params![task_id], |row| Ok(serde_json::json!({
            "id": row.get::<_, String>("id").ok(), "taskId": row.get::<_, String>("task_id").ok(),
            "charactersJson": row.get::<_, Option<String>>("characters_json").ok().flatten(),
            "plotOutline": row.get::<_, Option<String>>("plot_outline").ok().flatten(),
            "scriptBody": row.get::<_, Option<String>>("script_body").ok().flatten(),
            "hookOpening": row.get::<_, Option<String>>("hook_opening").ok().flatten(),
            "storyboardBase": row.get::<_, Option<String>>("storyboard_base").ok().flatten(),
            "rawResponse": row.get::<_, Option<String>>("raw_response").ok().flatten(),
            "createdAt": row.get::<_, String>("created_at").ok(),
        }))).unwrap().filter_map(|r| r.ok()).collect()
    };

    let review: Option<serde_json::Value> = conn.query_row(
        "SELECT * FROM review_records WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 1", params![task_id],
        |row| Ok(serde_json::json!({
            "id": row.get::<_, String>("id").ok(), "taskId": row.get::<_, String>("task_id").ok(),
            "score": row.get::<_, Option<i32>>("score").ok().flatten(), "status": row.get::<_, String>("status").ok(),
            "summary": row.get::<_, Option<String>>("summary").ok().flatten(),
            "issuesJson": row.get::<_, Option<String>>("issues_json").ok().flatten(),
            "suggestionsJson": row.get::<_, Option<String>>("suggestions_json").ok().flatten(),
            "dimensionsJson": row.get::<_, Option<String>>("dimensions_json").ok().flatten(),
            "reviewModel": row.get::<_, Option<String>>("review_model").ok().flatten(),
            "createdAt": row.get::<_, String>("created_at").ok(),
        })),
    ).ok();

    let assets: Vec<serde_json::Value> = {
        let mut stmt = conn.prepare("SELECT * FROM asset_records WHERE task_id = ?1 ORDER BY created_at").unwrap();
        stmt.query_map(params![task_id], |row| Ok(serde_json::json!({
            "id": row.get::<_, String>("id").ok(), "taskId": row.get::<_, String>("task_id").ok(),
            "assetType": row.get::<_, String>("asset_type").ok(),
            "assetDataJson": row.get::<_, Option<String>>("asset_data_json").ok().flatten(),
            "createdAt": row.get::<_, String>("created_at").ok(),
        }))).unwrap().filter_map(|r| r.ok()).collect()
    };

    let prompt_output: Option<serde_json::Value> = conn.query_row(
        "SELECT * FROM prompt_output_records WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 1", params![task_id],
        |row| Ok(serde_json::json!({
            "id": row.get::<_, String>("id").ok(), "taskId": row.get::<_, String>("task_id").ok(),
            "gridGroupsJson": row.get::<_, Option<String>>("grid_groups_json").ok().flatten(),
            "seedanceGroupsJson": row.get::<_, Option<String>>("seedance_groups_json").ok().flatten(),
            "generationModel": row.get::<_, Option<String>>("generation_model").ok().flatten(),
            "createdAt": row.get::<_, String>("created_at").ok(),
        })),
    ).ok();

    Some(serde_json::json!({ "task": task, "outputs": outputs, "review": review, "assets": assets, "promptOutput": prompt_output }))
}

// ── Delete Tasks ──

pub fn delete_script_task(conn: &Connection, task_id: &str) {
    for tbl in &["review_records", "script_outputs", "asset_records", "prompt_output_records", "seedance_analysis", "seedance_units"] {
        conn.execute(&format!("DELETE FROM {} WHERE task_id = ?1", tbl), params![task_id]).ok();
    }
    conn.execute("DELETE FROM script_tasks WHERE id = ?1", params![task_id]).ok();
}

pub fn delete_image_task(conn: &Connection, task_id: &str) {
    conn.execute("DELETE FROM image_review_records WHERE task_id = ?1", params![task_id]).ok();
    conn.execute("DELETE FROM image_outputs WHERE task_id = ?1", params![task_id]).ok();
    conn.execute("DELETE FROM image_tasks WHERE id = ?1", params![task_id]).ok();
}

pub fn delete_video_task(conn: &Connection, task_id: &str) {
    conn.execute("DELETE FROM video_review_records WHERE task_id = ?1", params![task_id]).ok();
    conn.execute("DELETE FROM video_outputs WHERE task_id = ?1", params![task_id]).ok();
    conn.execute("DELETE FROM video_tasks WHERE id = ?1", params![task_id]).ok();
}

// ── Projects ──

pub fn get_projects(conn: &Connection) -> Vec<serde_json::Value> {
    let mut stmt = conn.prepare("SELECT id, name, module_type, status, created_at, updated_at FROM projects ORDER BY updated_at DESC").unwrap();
    let rows: Vec<(String, String, String, String, String, String)> = stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
    }).unwrap().filter_map(|r| r.ok()).collect();

    let mut projects = Vec::new();
    for (id, name, module_type, status, _created_at, updated_at) in rows {
        let mut all_tasks: Vec<serde_json::Value> = Vec::new();

        let mut s = conn.prepare("SELECT st.id, st.mode, st.stage, st.updated_at, rr.score, rr.status FROM script_tasks st LEFT JOIN review_records rr ON rr.id = (SELECT inner_rr.id FROM review_records inner_rr WHERE inner_rr.task_id = st.id ORDER BY inner_rr.created_at DESC LIMIT 1) WHERE st.project_id = ?1 ORDER BY st.updated_at DESC").unwrap();
        s.query_map(params![id], |row| Ok(serde_json::json!({"taskId": row.get::<_,String>(0).unwrap_or_default(), "moduleType":"script", "mode": row.get::<_,String>(1).unwrap_or_default(), "stage": row.get::<_,String>(2).unwrap_or_default(), "updatedAt": row.get::<_,String>(3).unwrap_or_default(), "reviewScore": row.get::<_,Option<i32>>(4).ok().flatten(), "reviewStatus": row.get::<_,Option<String>>(5).ok().flatten() }))).unwrap().for_each(|r| { if let Ok(v) = r { all_tasks.push(v); } });

        let mut it = conn.prepare("SELECT id, mode, stage, updated_at FROM image_tasks WHERE project_id = ?1 ORDER BY updated_at DESC").unwrap();
        it.query_map(params![id], |row| Ok(serde_json::json!({"taskId": row.get::<_,String>(0).unwrap_or_default(), "moduleType":"image", "mode": row.get::<_,String>(1).unwrap_or_default(), "stage": row.get::<_,String>(2).unwrap_or_default(), "updatedAt": row.get::<_,String>(3).unwrap_or_default() }))).unwrap().for_each(|r| { if let Ok(v) = r { all_tasks.push(v); } });

        let mut vt = conn.prepare("SELECT id, mode, stage, updated_at FROM video_tasks WHERE project_id = ?1 ORDER BY updated_at DESC").unwrap();
        vt.query_map(params![id], |row| Ok(serde_json::json!({"taskId": row.get::<_,String>(0).unwrap_or_default(), "moduleType":"video", "mode": row.get::<_,String>(1).unwrap_or_default(), "stage": row.get::<_,String>(2).unwrap_or_default(), "updatedAt": row.get::<_,String>(3).unwrap_or_default() }))).unwrap().for_each(|r| { if let Ok(v) = r { all_tasks.push(v); } });

        all_tasks.sort_by(|a, b| b["updatedAt"].as_str().unwrap_or("").cmp(a["updatedAt"].as_str().unwrap_or("")));

        projects.push(serde_json::json!({
            "projectId": id, "projectName": name, "moduleType": module_type, "status": status,
            "taskCount": all_tasks.len(), "latestDate": all_tasks.first().and_then(|t| t["updatedAt"].as_str()).unwrap_or(&updated_at),
            "tasks": all_tasks,
        }));
    }
    projects
}

pub fn rename_project(conn: &Connection, project_id: &str, new_name: &str) {
    conn.execute("UPDATE projects SET name = ?1, updated_at = ?2 WHERE id = ?3", params![new_name.trim(), now(), project_id]).ok();
}

pub fn delete_project(conn: &Connection, project_id: &str) {
    let script_tids: Vec<String> = conn.prepare("SELECT id FROM script_tasks WHERE project_id = ?1").unwrap()
        .query_map(params![project_id], |row| row.get::<_,String>(0)).unwrap().filter_map(|r| r.ok()).collect();
    let image_tids: Vec<String> = conn.prepare("SELECT id FROM image_tasks WHERE project_id = ?1").unwrap()
        .query_map(params![project_id], |row| row.get::<_,String>(0)).unwrap().filter_map(|r| r.ok()).collect();
    let video_tids: Vec<String> = conn.prepare("SELECT id FROM video_tasks WHERE project_id = ?1").unwrap()
        .query_map(params![project_id], |row| row.get::<_,String>(0)).unwrap().filter_map(|r| r.ok()).collect();

    for tid in &script_tids {
        for tbl in &["review_records", "script_outputs", "asset_records", "prompt_output_records", "seedance_analysis", "seedance_units", "script_tasks"] {
            conn.execute(&format!("DELETE FROM {} WHERE task_id = ?1", tbl), params![tid]).ok();
        }
    }
    for tid in &image_tids {
        conn.execute("DELETE FROM image_review_records WHERE task_id = ?1", params![tid]).ok();
        conn.execute("DELETE FROM image_outputs WHERE task_id = ?1", params![tid]).ok();
        conn.execute("DELETE FROM image_tasks WHERE id = ?1", params![tid]).ok();
    }
    for tid in &video_tids {
        conn.execute("DELETE FROM video_review_records WHERE task_id = ?1", params![tid]).ok();
        conn.execute("DELETE FROM video_outputs WHERE task_id = ?1", params![tid]).ok();
        conn.execute("DELETE FROM video_tasks WHERE id = ?1", params![tid]).ok();
    }
    conn.execute("DELETE FROM projects WHERE id = ?1", params![project_id]).ok();
}

// ── Asset Records ──

pub fn get_assets_by_task(conn: &Connection, task_id: &str) -> Vec<serde_json::Value> {
    let mut stmt = conn.prepare("SELECT id, asset_type, asset_data_json, created_at FROM asset_records WHERE task_id = ?1 ORDER BY created_at").unwrap();
    stmt.query_map(params![task_id], |row| Ok(serde_json::json!({"id": row.get::<_,String>(0).unwrap_or_default(), "assetType": row.get::<_,String>(1).unwrap_or_default(), "assetDataJson": row.get::<_,Option<String>>(2).ok().flatten(), "createdAt": row.get::<_,String>(3).unwrap_or_default() }))).unwrap().filter_map(|r| r.ok()).collect()
}

pub fn update_assets(conn: &Connection, task_id: &str, characters: &str, scenes: &str, props: &str) {
    conn.execute("DELETE FROM asset_records WHERE task_id = ?1", params![task_id]).ok();
    let t = now();
    for (ty, val) in [("characters", characters), ("scenes", scenes), ("props", props)] {
        conn.execute("INSERT INTO asset_records (id, task_id, asset_type, asset_data_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![uuid(), task_id, ty, val, t]).ok();
    }
}

// ── Prompt Output ──

pub fn get_prompt_output_by_task(conn: &Connection, task_id: &str) -> Option<serde_json::Value> {
    conn.query_row("SELECT * FROM prompt_output_records WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 1", params![task_id], |row| {
        Ok(serde_json::json!({"id": row.get::<_,String>("id").ok(), "taskId": row.get::<_,String>("task_id").ok(),
            "gridGroupsJson": row.get::<_,Option<String>>("grid_groups_json").ok().flatten(),
            "seedanceGroupsJson": row.get::<_,Option<String>>("seedance_groups_json").ok().flatten(),
            "generationModel": row.get::<_,Option<String>>("generation_model").ok().flatten(),
            "createdAt": row.get::<_,String>("created_at").ok()}))
    }).ok()
}

pub fn update_prompt_output(conn: &Connection, task_id: &str, seedance_groups: &str) {
    conn.execute("UPDATE prompt_output_records SET seedance_groups_json = ?1 WHERE task_id = ?2", params![seedance_groups, task_id]).ok();
}

// ── Generation & Review (mock wrappers) ──

pub fn run_image_generation(conn: &Connection, input: &serde_json::Value) -> serde_json::Value {
    let task_id = input["taskId"].as_str().unwrap_or("");
    let t = now();
    conn.execute("UPDATE image_tasks SET stage = 'ready', updated_at = ?1 WHERE id = ?2", params![t, task_id]).ok();
    conn.execute("INSERT INTO image_outputs (id, task_id, sections_json, raw_response, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![uuid(), task_id, "[]", "{}", t]).ok();
    serde_json::json!({"sections": [], "taskId": task_id, "generatedAt": t})
}

pub fn run_video_generation(conn: &Connection, input: &serde_json::Value) -> serde_json::Value {
    let task_id = input["taskId"].as_str().unwrap_or("");
    let t = now();
    conn.execute("UPDATE video_tasks SET stage = 'ready', updated_at = ?1 WHERE id = ?2", params![t, task_id]).ok();
    conn.execute("INSERT INTO video_outputs (id, task_id, sections_json, raw_response, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![uuid(), task_id, "[]", "{}", t]).ok();
    serde_json::json!({"sections": [], "taskId": task_id, "generatedAt": t})
}

pub fn run_image_review(conn: &Connection, input: &serde_json::Value) -> serde_json::Value {
    let task_id = input["taskId"].as_str().unwrap_or("");
    let t = now();
    conn.execute("INSERT INTO image_review_records (id, task_id, score, status, summary, created_at) VALUES (?1, ?2, 85, 'passed', '自动审核通过', ?3)", params![uuid(), task_id, t]).ok();
    serde_json::json!({"score": 85, "status": "passed"})
}

pub fn run_video_review(conn: &Connection, input: &serde_json::Value) -> serde_json::Value {
    let task_id = input["taskId"].as_str().unwrap_or("");
    let t = now();
    conn.execute("INSERT INTO video_review_records (id, task_id, score, status, summary, created_at) VALUES (?1, ?2, 85, 'passed', '自动审核通过', ?3)", params![uuid(), task_id, t]).ok();
    serde_json::json!({"score": 85, "status": "passed"})
}

pub fn run_script_review(conn: &Connection, input: &serde_json::Value) -> Result<serde_json::Value, String> {
    let task_id = input["taskId"].as_str().unwrap_or("").to_string();
    tokio::runtime::Handle::current()
        .block_on(crate::services::script_review::run_script_review(conn, &task_id))
}

// ── Screenplay Finalize ──

#[derive(Debug, Deserialize)]
pub struct FinalizeScreenplayInput {
    pub project_name: String,
    pub duration: String,
    pub concept: Option<String>,
    pub scenes: Vec<serde_json::Value>,
    pub doctor: Option<serde_json::Value>,
    pub linked_script_task_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FinalizeScreenplayResult {
    pub project_id: String,
    pub task_id: String,
    pub was_create: bool,
}

pub fn finalize_screenplay(conn: &Connection, input: &FinalizeScreenplayInput) -> FinalizeScreenplayResult {
    let t = now();
    let task_id = uuid();
    let project_id = input.linked_script_task_id.as_ref().and_then(|linked| {
        conn.query_row("SELECT project_id FROM script_tasks WHERE id = ?1", params![linked], |row| row.get::<_,String>(0)).ok()
    }).unwrap_or_else(|| {
        let pid = uuid();
        conn.execute("INSERT INTO projects (id, name, module_type, status, created_at, updated_at) VALUES (?1, ?2, 'script', 'active', ?3, ?4)", params![pid, input.project_name, t, t]).ok();
        pid
    });

    conn.execute("INSERT INTO script_tasks (id, project_id, mode, input_summary, duration, stage, created_at, updated_at) VALUES (?1, ?2, 'plot', ?3, ?4, 'ready', ?5, ?6) ON CONFLICT(id) DO UPDATE SET stage = 'ready', updated_at = ?7",
        params![task_id, project_id, input.concept.as_deref().unwrap_or_default(), input.duration, t, t, t]).ok();

    let script_body: String = input.scenes.iter().map(|s| {
        format!("{}\n{}", s["header"].as_str().unwrap_or(""), s["body"].as_str().unwrap_or(""))
    }).collect::<Vec<_>>().join("\n\n");
    let raw = serde_json::json!({"scenes": input.scenes, "doctor": input.doctor});

    conn.execute("INSERT INTO script_outputs (id, task_id, characters_json, plot_outline, script_body, raw_response, created_at) VALUES (?1, ?2, '[]', ?3, ?4, ?5, ?6)",
        params![uuid(), task_id, script_body, script_body, raw.to_string(), t]).ok();

    let was_create = input.linked_script_task_id.is_none();
    FinalizeScreenplayResult { project_id, task_id, was_create }
}

// ── Seedance ──

pub fn seedance_phase_ad(conn: &Connection, task_id: &str) -> Result<serde_json::Value, String> {
    let tid = task_id.to_string();
    let analysis = tokio::runtime::Handle::current()
        .block_on(crate::services::seedance_service::run_phase_ad(conn, &tid))?;
    Ok(serde_json::to_value(&analysis).map_err(|e| e.to_string())?)
}

pub fn seedance_get_analysis(conn: &Connection, task_id: &str) -> Option<serde_json::Value> {
    conn.query_row("SELECT * FROM seedance_analysis WHERE task_id = ?1", params![task_id], |row| {
        Ok(serde_json::json!({"taskId": row.get::<_,String>("task_id").ok(), "paragraphIndexJson": row.get::<_,Option<String>>("paragraph_index_json").ok().flatten(), "structureType": row.get::<_,Option<String>>("structure_type").ok().flatten(), "emotionMapJson": row.get::<_,Option<String>>("emotion_map_json").ok().flatten(), "unitsPlanJson": row.get::<_,Option<String>>("units_plan_json").ok().flatten(), "totalSec": row.get::<_,Option<i32>>("total_sec").ok().flatten(), "totalUnits": row.get::<_,Option<i32>>("total_units").ok().flatten(), "createdAt": row.get::<_,String>("created_at").ok(), "updatedAt": row.get::<_,String>("updated_at").ok()}))
    }).ok()
}

pub fn seedance_list_units(conn: &Connection, task_id: &str) -> Vec<serde_json::Value> {
    let mut stmt = conn.prepare("SELECT * FROM seedance_units WHERE task_id = ?1 ORDER BY unit_index").unwrap();
    stmt.query_map(params![task_id], |row| Ok(serde_json::json!({"id": row.get::<_,String>("id").ok(), "taskId": row.get::<_,String>("task_id").ok(), "unitIndex": row.get::<_,i32>("unit_index").ok(), "durationSec": row.get::<_,Option<i32>>("duration_sec").ok().flatten(), "sceneType": row.get::<_,Option<String>>("scene_type").ok().flatten(), "subShotCount": row.get::<_,Option<i32>>("sub_shot_count").ok().flatten(), "copyArea": row.get::<_,Option<String>>("copy_area").ok().flatten(), "noteAreaJson": row.get::<_,Option<String>>("note_area_json").ok().flatten(), "status": row.get::<_,String>("status").ok(), "retryCount": row.get::<_,i32>("retry_count").ok(), "errorMessage": row.get::<_,Option<String>>("error_message").ok().flatten(), "createdAt": row.get::<_,String>("created_at").ok(), "updatedAt": row.get::<_,String>("updated_at").ok() }))).unwrap().filter_map(|r| r.ok()).collect()
}

pub fn seedance_get_unit(conn: &Connection, task_id: &str, unit_index: i32) -> Option<serde_json::Value> {
    conn.query_row("SELECT * FROM seedance_units WHERE task_id = ?1 AND unit_index = ?2", params![task_id, unit_index], |row| {
        Ok(serde_json::json!({"id": row.get::<_,String>("id").ok(), "taskId": row.get::<_,String>("task_id").ok(), "unitIndex": row.get::<_,i32>("unit_index").ok(), "durationSec": row.get::<_,Option<i32>>("duration_sec").ok().flatten(), "sceneType": row.get::<_,Option<String>>("scene_type").ok().flatten(), "subShotCount": row.get::<_,Option<i32>>("sub_shot_count").ok().flatten(), "copyArea": row.get::<_,Option<String>>("copy_area").ok().flatten(), "noteAreaJson": row.get::<_,Option<String>>("note_area_json").ok().flatten(), "status": row.get::<_,String>("status").ok(), "retryCount": row.get::<_,i32>("retry_count").ok(), "errorMessage": row.get::<_,Option<String>>("error_message").ok().flatten(), "createdAt": row.get::<_,String>("created_at").ok(), "updatedAt": row.get::<_,String>("updated_at").ok() }))
    }).ok()
}

pub fn seedance_run_unit(conn: &Connection, task_id: &str, unit_index: i32) -> Result<serde_json::Value, String> {
    let tid = task_id.to_string();
    tokio::runtime::Handle::current()
        .block_on(crate::services::seedance_service::run_unit_generation(conn, &tid, unit_index as usize))
}

pub fn seedance_run_all(conn: &Connection, task_id: &str) -> Result<serde_json::Value, String> {
    let tid = task_id.to_string();
    let results = tokio::runtime::Handle::current()
        .block_on(crate::services::seedance_service::run_generate_all(conn, &tid, None))?;
    Ok(serde_json::json!({"taskId": task_id, "completed": true, "results": results}))
}

// ── Prompt Generation ──

pub fn run_prompt_generation(conn: &Connection, input: &serde_json::Value) -> Result<serde_json::Value, String> {
    tokio::runtime::Handle::current()
        .block_on(crate::services::prompt_generation::run_prompt_generation(conn, input))
}

pub fn run_prompt_group_gen(conn: &Connection, input: &serde_json::Value) -> Result<serde_json::Value, String> {
    tokio::runtime::Handle::current()
        .block_on(crate::services::prompt_generation::run_prompt_group_generation(conn, input))
}

pub fn get_scene_count(conn: &Connection, task_id: &str) -> Option<i64> {
    conn.query_row("SELECT json_array_length(grid_groups_json) FROM prompt_output_records WHERE task_id = ?1 LIMIT 1", params![task_id], |row| row.get(0)).ok()
}

pub fn get_segment_titles(conn: &Connection, task_id: &str) -> Vec<String> {
    let s: Option<String> = conn.query_row("SELECT grid_groups_json FROM prompt_output_records WHERE task_id = ?1 LIMIT 1", params![task_id], |row| row.get(0)).ok();
    s.and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(&s).ok()).map(|v| v.iter().filter_map(|g| g["title"].as_str().map(String::from)).collect()).unwrap_or_default()
}

pub fn run_quality_check(_conn: &Connection, _task_id: &str) -> serde_json::Value {
    serde_json::json!({"passed": true, "issues": [], "score": 90})
}

pub fn generate_outline(conn: &Connection, input: &serde_json::Value) -> Result<serde_json::Value, String> {
    tokio::runtime::Handle::current()
        .block_on(crate::services::prompt_generation::generate_outline(conn, input))
}

pub fn confirm_outline(conn: &Connection, input: &serde_json::Value) {
    crate::services::prompt_generation::confirm_outline(conn, input);
}

pub fn get_outline(conn: &Connection, task_id: &str) -> Option<serde_json::Value> {
    crate::services::prompt_generation::get_outline(conn, task_id)
}

// ── Asset Extraction ──

pub fn run_asset_extraction(conn: &Connection, input: &serde_json::Value) -> Result<serde_json::Value, String> {
    let task_id = input["taskId"].as_str().unwrap_or("").to_string();
    tokio::runtime::Handle::current()
        .block_on(crate::services::asset_extraction::run_asset_extraction(conn, &task_id))
}
