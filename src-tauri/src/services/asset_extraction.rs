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

fn extract_json_array(text: &str) -> Option<Vec<serde_json::Value>> {
    let trimmed = text.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(arr) = v.as_array() {
            return Some(arr.clone());
        }
        if let Some(obj) = v.as_object() {
            for val in obj.values() {
                if let Some(arr) = val.as_array() {
                    if !arr.is_empty() {
                        return Some(arr.clone());
                    }
                }
            }
        }
    }
    if let Some(start) = trimmed.find('[') {
        if let Some(end) = trimmed.rfind(']') {
            let slice = &trimmed[start..=end];
            if let Ok(v) = serde_json::from_str::<Vec<serde_json::Value>>(slice) {
                return Some(v);
            }
        }
    }
    None
}

fn normalize_character(raw: &serde_json::Value) -> serde_json::Value {
    json!({
        "id": uuid(),
        "name": raw.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown"),
        "role": raw.get("role").and_then(|v| v.as_str()).unwrap_or("Unknown"),
        "aliases": raw.get("aliases").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>()).unwrap_or_default(),
        "appearance": raw.get("appearance").and_then(|v| v.as_str()).unwrap_or(""),
        "clothing": raw.get("clothing").and_then(|v| v.as_str()).unwrap_or(""),
        "personality": raw.get("personality").and_then(|v| v.as_str()).unwrap_or(""),
        "colorPalette": raw.get("colorPalette").and_then(|v| v.as_str()).unwrap_or(""),
        "visualAnchor": raw.get("visualAnchor").and_then(|v| v.as_str()).unwrap_or(""),
        "aiPrompt": raw.get("aiPrompt").and_then(|v| v.as_str()).unwrap_or(""),
    })
}

fn normalize_scene(raw: &serde_json::Value) -> serde_json::Value {
    json!({
        "id": uuid(),
        "name": raw.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown"),
        "aliases": raw.get("aliases").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>()).unwrap_or_default(),
        "timeOfDay": raw.get("timeOfDay").and_then(|v| v.as_str()).unwrap_or(""),
        "atmosphere": raw.get("atmosphere").and_then(|v| v.as_str()).unwrap_or(""),
        "materials": raw.get("materials").and_then(|v| v.as_str()).unwrap_or(""),
        "landmarks": raw.get("landmarks").and_then(|v| v.as_str()).unwrap_or(""),
        "colorTemperature": raw.get("colorTemperature").and_then(|v| v.as_str()).unwrap_or(""),
        "visualAnchor": raw.get("visualAnchor").and_then(|v| v.as_str()).unwrap_or(""),
        "aiPrompt": raw.get("aiPrompt").and_then(|v| v.as_str()).unwrap_or(""),
    })
}

fn normalize_prop(raw: &serde_json::Value) -> serde_json::Value {
    json!({
        "id": uuid(),
        "name": raw.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown"),
        "aliases": raw.get("aliases").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>()).unwrap_or_default(),
        "dramaticFunction": raw.get("dramaticFunction").and_then(|v| v.as_str()).unwrap_or(""),
        "form": raw.get("form").and_then(|v| v.as_str()).unwrap_or(""),
        "material": raw.get("material").and_then(|v| v.as_str()).unwrap_or(""),
        "surfaceState": raw.get("surfaceState").and_then(|v| v.as_str()).unwrap_or(""),
        "visualAnchor": raw.get("visualAnchor").and_then(|v| v.as_str()).unwrap_or(""),
        "aiPrompt": raw.get("aiPrompt").and_then(|v| v.as_str()).unwrap_or(""),
    })
}

/// Regex-based character extraction from script text (local fallback).
fn extract_characters_from_script(script_body: &str) -> Vec<serde_json::Value> {
    let re = regex_lite::Regex::new(r"\*\*(.{1,8})\*\*(?:\s*(?:（[^）]*）))?\s*[：:]").unwrap();
    let mut names: Vec<(String, String)> = Vec::new();
    for cap in re.captures_iter(script_body) {
        let name = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("").to_string();
        if name.is_empty() || name.starts_with("场景") || name.starts_with("出场") {
            continue;
        }
        if names.iter().any(|(n, _)| n == &name) {
            continue;
        }
        let pos = cap.get(0).map(|m| m.start()).unwrap_or(0);
        let before = &script_body[if pos > 100 { pos - 100 } else { 0 }..pos];
        let role = if before.contains("女主") { "女主" } else if before.contains("男主") { "男主" } else if before.contains("反派") { "反派" } else { "" };
        names.push((name, role.to_string()));
    }
    names
        .into_iter()
        .map(|(name, role)| {
            json!({
                "id": uuid(),
                "name": name,
                "role": role,
                "aliases": [],
                "appearance": "",
                "clothing": "",
                "personality": "",
                "colorPalette": "",
                "visualAnchor": "",
                "aiPrompt": "",
            })
        })
        .collect()
}

/// Regex-based scene extraction from script text (local fallback).
fn extract_scenes_from_script(script_body: &str) -> Vec<serde_json::Value> {
    let re = regex_lite::Regex::new(r"\*\*场景[：:]\*\*\s*([^\n]+)").unwrap();
    let mut scenes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for cap in re.captures_iter(script_body) {
        let raw = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        if raw.is_empty() || seen.contains(raw) {
            continue;
        }
        seen.insert(raw.to_string());
        let time_re = regex_lite::Regex::new(r"[·・]\s*(日|夜|晨|黄昏|傍晚|午|晚)").unwrap();
        let time_of_day = time_re
            .captures(raw)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        scenes.push(json!({
            "id": uuid(),
            "name": raw,
            "aliases": [],
            "timeOfDay": time_of_day,
            "atmosphere": "",
            "materials": "",
            "landmarks": "",
            "colorTemperature": "",
            "visualAnchor": "",
            "aiPrompt": "",
        }));
    }
    scenes
}

fn extract_props_from_script(_script_body: &str) -> Vec<serde_json::Value> {
    vec![]
}

fn persist_assets(conn: &Connection, task_id: &str, characters: &[serde_json::Value], scenes: &[serde_json::Value], props: &[serde_json::Value], time: &str) {
    conn.execute("DELETE FROM asset_records WHERE task_id = ?1", params![task_id]).ok();
    for c in characters {
        conn.execute(
            "INSERT INTO asset_records (id, task_id, asset_type, asset_data_json, created_at) VALUES (?1, ?2, 'character', ?3, ?4)",
            params![uuid(), task_id, c.to_string(), time],
        )
        .ok();
    }
    for s in scenes {
        conn.execute(
            "INSERT INTO asset_records (id, task_id, asset_type, asset_data_json, created_at) VALUES (?1, ?2, 'scene', ?3, ?4)",
            params![uuid(), task_id, s.to_string(), time],
        )
        .ok();
    }
    for p in props {
        conn.execute(
            "INSERT INTO asset_records (id, task_id, asset_type, asset_data_json, created_at) VALUES (?1, ?2, 'prop', ?3, ?4)",
            params![uuid(), task_id, p.to_string(), time],
        )
        .ok();
    }
}

/// Try LLM extraction with silent retry.
async fn try_extract_array(
    runtime_config: &RuntimeConfig,
    slug: &str,
    script_body: &str,
) -> Option<Vec<serde_json::Value>> {
    for _attempt in 0..2 {
        let result = server_proxy::request_server_llm(ServerLlmParams {
            runtime_config: RuntimeConfig { ..runtime_config.clone() },
            prompt_slug: slug.to_string(),
            temperature: Some(0.2),
            user_messages: vec![json!({"role": "user", "content": script_body})],
        })
        .await;
        match result {
            Ok(text) => {
                let parsed = extract_json_array(&text);
                if let Some(arr) = parsed {
                    if !arr.is_empty() {
                        return Some(arr);
                    }
                }
            }
            Err(_) => continue,
        }
    }
    None
}

fn load_script_body(conn: &Connection, task_id: &str) -> Result<String, String> {
    let row: Result<(Option<String>,), _> = conn.query_row(
        "SELECT script_body FROM script_outputs WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 1",
        params![task_id],
        |row| Ok((row.get(0)?,)),
    );
    match row {
        Ok((Some(body),)) if !body.trim().is_empty() => Ok(body),
        _ => {
            let task_exists: bool = conn
                .query_row("SELECT 1 FROM script_tasks WHERE id = ?1", params![task_id], |_| Ok(true))
                .unwrap_or(false);
            if !task_exists {
                Err("找不到该任务，请重新选择或重新创建剧本任务。".into())
            } else {
                Err("当前剧本任务还没有正文内容。请先在剧本阶段生成剧本，或使用「导入已有剧本」功能上传剧本后再继续。".into())
            }
        }
    }
}

/// Main entry: run asset extraction (LLM + regex fallback).
pub async fn run_asset_extraction(
    conn: &Connection,
    task_id: &str,
) -> Result<serde_json::Value, String> {
    let time = now();
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

    let script_body = load_script_body(conn, task_id)?;
    let mut fallback_used = false;

    // Characters
    let characters = if let Some(parsed) = try_extract_array(&runtime_config, "asset_character", &script_body).await {
        parsed.iter().map(normalize_character).collect()
    } else {
        vec![]
    };
    let characters = if !characters.is_empty() {
        characters
    } else {
        let extracted = extract_characters_from_script(&script_body);
        if !extracted.is_empty() {
            fallback_used = true;
        }
        extracted
    };

    // Scenes
    let scenes = if let Some(parsed) = try_extract_array(&runtime_config, "asset_scene", &script_body).await {
        parsed.iter().map(normalize_scene).collect()
    } else {
        vec![]
    };
    let scenes = if !scenes.is_empty() {
        scenes
    } else {
        let extracted = extract_scenes_from_script(&script_body);
        if !extracted.is_empty() {
            fallback_used = true;
        }
        extracted
    };

    // Props
    let props = if let Some(parsed) = try_extract_array(&runtime_config, "asset_prop", &script_body).await {
        parsed.iter().map(normalize_prop).collect()
    } else {
        vec![]
    };
    let props = if !props.is_empty() {
        props
    } else {
        extract_props_from_script(&script_body)
    };

    persist_assets(conn, task_id, &characters, &scenes, &props, &time);

    let extraction_model = if fallback_used {
        format!("workfisher-asset-extractor-v2 / {} / regex-fallback", runtime_config.default_model)
    } else {
        format!("workfisher-asset-extractor-v2 / {}", runtime_config.default_model)
    };

    Ok(json!({
        "taskId": task_id,
        "characters": characters,
        "scenes": scenes,
        "props": props,
        "extractedAt": time,
        "extractionModel": extraction_model,
        "fallbackUsed": fallback_used,
    }))
}

/// Update assets directly (from user edit).
pub fn update_assets(conn: &Connection, task_id: &str, characters: &str, scenes: &str, props: &str) {
    crate::db::crud::update_assets(conn, task_id, characters, scenes, props);
}
