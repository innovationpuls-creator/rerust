use rusqlite::Connection;

use crate::db::crud;
use crate::llm::config::RuntimeConfig;
use crate::llm::server_proxy;
use crate::services::screenplay_store;
use crate::utils::step_parser;

// ── Skill Status ──

pub fn skill_status() -> serde_json::Value {
    serde_json::json!({
        "cached": true, "cacheDir": "(server-managed)", "main": true, "core": true,
        "formatUltrashort": true, "formatShort": true, "chinese": true, "craft": true,
        "aiPitfalls": true, "checkpointTemplate": true, "genreHookLibrary": true,
    })
}

// ── Public API for IPC handlers ──

pub fn create_project(init: screenplay_store::ProjectInit) -> screenplay_store::ProjectRecord {
    screenplay_store::create_project(init)
}

pub fn get_project(project_id: &str) -> Option<screenplay_store::ProjectRecord> {
    screenplay_store::load_project(project_id)
}

pub fn list_recent_projects(limit: usize) -> Vec<serde_json::Value> {
    screenplay_store::list_recent_projects(limit)
}

pub fn delete_project(project_id: &str) -> bool {
    screenplay_store::delete_project_file(project_id)
}

pub fn update_step_structured(
    project_id: &str,
    step_number: u8,
    structured: serde_json::Value,
) -> bool {
    screenplay_store::update_active_step_structured(project_id, step_number, structured)
}

pub fn rename_project(project_id: &str, new_name: &str) -> bool {
    screenplay_store::rename_project(project_id, new_name)
}

pub fn finalize_to_script_task(
    conn: &Connection,
    project_id: &str,
) -> Result<serde_json::Value, String> {
    let rec = screenplay_store::load_project(project_id).ok_or("Project not found")?;

    let scenes = if rec.init.path.as_deref() == Some("import") {
        let imported_script = rec.init.imported_script.clone().unwrap_or_default();
        if imported_script.trim().is_empty() {
            return Err("Imported script is empty".into());
        }
        let re = regex_lite::Regex::new(r"【\s*(场景|场)\s*[一二三四五六七八九十百零\d]+\s*[:：][^】]+】\s*[（(][^)）]*[)）]").unwrap();
        let matches: Vec<_> = re.find_iter(&imported_script).collect();

        if matches.len() >= 2 {
            matches
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let start = m.start();
                    let end = if i + 1 < matches.len() {
                        matches[i + 1].start()
                    } else {
                        imported_script.len()
                    };
                    let block = imported_script[start..end].trim().to_string();
                    let header = block.lines().next().unwrap_or("").to_string();
                    let body = block[header.len()..].trim().to_string();
                    let dur_re = regex_lite::Regex::new(r"[（(][^)）]*?(\d+)[^)）]*?[)）]").unwrap();
                    let dur = dur_re
                        .captures(&header)
                        .and_then(|c| c.get(1))
                        .map(|m| format!("约 {} 秒", m.as_str()))
                        .unwrap_or_else(|| "约 30 秒".into());
                    serde_json::json!({"index": i+1, "header": header, "duration": dur, "plotRhythm": "中", "emotionRhythm": "中", "body": body})
                })
                .collect()
        } else {
            let file_name = rec.init.imported_file_name.clone();
            let header = file_name
                .map(|n| format!("【导入剧本：{}】", n))
                .unwrap_or_else(|| "【导入剧本】".into());
            vec![serde_json::json!({
                "index": 1, "header": header, "duration": rec.init.duration.clone().unwrap_or_else(|| "未知".into()),
                "plotRhythm": "中", "emotionRhythm": "中", "body": imported_script.trim(),
            })]
        }
    } else {
        let step7 = screenplay_store::get_active_version(project_id, 7)
            .ok_or("Step 7 output not found")?;
        let structured = step7.structured.ok_or("Step 7 structured data missing")?;
        let scenes = structured["scenes"]
            .as_array()
            .ok_or("Step 7 scenes is not an array")?
            .clone();
        scenes
    };

    let doctor = screenplay_store::get_active_version(project_id, 8)
        .and_then(|v| v.structured);

    let project_name = rec
        .init
        .name
        .clone()
        .or_else(|| rec.init.concept.as_ref().map(|c| c.chars().take(30).collect()))
        .unwrap_or_else(|| "未命名剧本".into());

    let result = {
        crud::finalize_screenplay(
            conn,
            &crud::FinalizeScreenplayInput {
                project_name,
                duration: rec.init.duration.clone().unwrap_or_else(|| "2分钟".into()),
                concept: rec.init.concept.clone(),
                scenes,
                doctor,
                linked_script_task_id: rec.linked_script_task_id.clone(),
            },
        )
    };

    screenplay_store::set_linked_script_task_id(project_id, &result.task_id);

    Ok(serde_json::json!({
        "projectId": result.project_id,
        "scriptTaskId": result.task_id,
        "wasCreate": result.was_create,
    }))
}

pub async fn generate_step(
    conn: &Connection,
    project_id: &str,
    step_number: u8,
    user_feedback: Option<String>,
    on_chunk: impl Fn(&str),
) -> Result<serde_json::Value, String> {
    let settings = crud::get_app_settings(conn);

    let runtime_config = RuntimeConfig {
        api_key: settings.text_key,
        api_base_url: settings.text_endpoint,
        default_model: settings.text_model,
        text_mode: settings.text_mode,
        mode: String::new(),
        image_endpoint: String::new(),
        image_key: String::new(),
        image_model: String::new(),
        review_threshold: 90,
        enable_local_save: true,
    };

    if runtime_config.api_key.is_empty() || runtime_config.api_base_url.is_empty() {
        return Err("API 未配置, 请先到设置页填写文字模型 API 地址和密钥.".into());
    }

    let rec = screenplay_store::load_project(project_id).ok_or("Project not found")?;
    let project_snapshot = screenplay_store::build_project_snapshot(project_id);

    let params = server_proxy::ContextualLlmParams {
        runtime_config,
        context_type: "screenplay_step".into(),
        context_params: serde_json::json!({
            "stepNumber": step_number,
            "init": rec.init,
            "projectSnapshot": project_snapshot,
            "userFeedback": user_feedback,
        }),
        temperature: None,
        max_tokens_override: None,
    };

    let mut full_text = String::new();
    server_proxy::request_contextual_llm_stream(params, |chunk| {
        full_text.push_str(chunk);
        on_chunk(chunk);
    })
    .await
    .map_err(|e| e.to_string())?;

    let structured = parse_step_output(step_number, &full_text);

    let version = screenplay_store::append_version(
        project_id,
        step_number,
        user_feedback
            .as_ref()
            .map(|fb| format!("修改: {}", &fb[..fb.len().min(20)]))
            .or_else(|| Some("初版".into())),
        Some(full_text.clone()),
        structured.clone(),
        user_feedback,
    );

    Ok(serde_json::json!({
        "versionId": version.id,
        "text": full_text,
        "structured": structured,
    }))
}

pub async fn generate_step_async(
    settings: serde_json::Value,
    project_id: &str,
    step_number: u8,
    user_feedback: Option<String>,
    on_chunk: impl Fn(&str),
) -> Result<serde_json::Value, String> {
    let runtime_config = RuntimeConfig {
        api_key: settings["textKey"].as_str().unwrap_or_default().to_string(),
        api_base_url: settings["textEndpoint"].as_str().unwrap_or_default().to_string(),
        default_model: settings["textModel"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("deepseek-chat")
            .to_string(),
        text_mode: settings["textMode"].as_str().unwrap_or("openai").to_string(),
        mode: String::new(),
        image_endpoint: String::new(),
        image_key: String::new(),
        image_model: String::new(),
        review_threshold: settings["reviewThreshold"].as_i64().unwrap_or(90) as u8,
        enable_local_save: settings["enableLocalSave"].as_bool().unwrap_or(false),
    };

    if runtime_config.api_key.is_empty() || runtime_config.api_base_url.is_empty() {
        return Err("API 未配置, 请先到设置页填写文字模型 API 地址和密钥.".into());
    }

    let rec = screenplay_store::load_project(project_id).ok_or("Project not found")?;
    let project_snapshot = screenplay_store::build_project_snapshot(project_id);

    let params = server_proxy::ContextualLlmParams {
        runtime_config,
        context_type: "screenplay_step".into(),
        context_params: serde_json::json!({
            "stepNumber": step_number,
            "init": rec.init,
            "projectSnapshot": project_snapshot,
            "userFeedback": user_feedback,
        }),
        temperature: None,
        max_tokens_override: None,
    };

    let mut full_text = String::new();
    server_proxy::request_contextual_llm_stream(params, |chunk| {
        full_text.push_str(chunk);
        on_chunk(chunk);
    })
    .await
    .map_err(|e| e.to_string())?;

    let structured = parse_step_output(step_number, &full_text);

    let _version = screenplay_store::append_version(
        project_id,
        step_number,
        user_feedback
            .as_ref()
            .map(|fb| format!("修改: {}", &fb[..fb.len().min(20)]))
            .or_else(|| Some("初版".into())),
        Some(full_text.clone()),
        structured.clone(),
        user_feedback,
    );

    Ok(serde_json::json!({
        "versionId": _version.id,
        "text": full_text,
        "structured": structured,
    }))
}

pub async fn selfcheck_step(
    conn: &Connection,
    project_id: &str,
    step_number: u8,
    on_chunk: impl Fn(&str),
) -> Result<serde_json::Value, String> {
    let rec = screenplay_store::load_project(project_id).ok_or("Project not found")?;

    let settings = crud::get_app_settings(conn);

    let runtime_config = RuntimeConfig {
        api_key: settings.text_key,
        api_base_url: settings.text_endpoint,
        default_model: settings.text_model,
        text_mode: settings.text_mode,
        mode: String::new(),
        image_endpoint: String::new(),
        image_key: String::new(),
        image_model: String::new(),
        review_threshold: 90,
        enable_local_save: true,
    };

    let active = screenplay_store::get_active_version(project_id, step_number);
    let current_output = active
        .as_ref()
        .and_then(|v| v.output.clone())
        .unwrap_or_else(|| "(无产出)".into());
    let current_selection = rec.selections.get(&step_number.to_string()).cloned();

    let params = server_proxy::ContextualLlmParams {
        runtime_config,
        context_type: "screenplay_selfcheck".into(),
        context_params: serde_json::json!({
            "stepNumber": step_number,
            "init": rec.init,
            "currentOutput": current_output,
            "currentSelection": current_selection,
        }),
        temperature: None,
        max_tokens_override: None,
    };

    let mut full_text = String::new();
    server_proxy::request_contextual_llm_stream(params, |chunk| {
        full_text.push_str(chunk);
        on_chunk(chunk);
    })
    .await
    .map_err(|e| e.to_string())?;

    let items = parse_selfcheck(&full_text);
    screenplay_store::save_selfcheck(project_id, step_number, items.clone());

    Ok(serde_json::json!({ "items": items }))
}

pub async fn selfcheck_step_async(
    settings: serde_json::Value,
    project_id: &str,
    step_number: u8,
    on_chunk: impl Fn(&str),
) -> Result<serde_json::Value, String> {
    let rec = screenplay_store::load_project(project_id).ok_or("Project not found")?;

    let runtime_config = RuntimeConfig {
        api_key: settings["textKey"].as_str().unwrap_or_default().to_string(),
        api_base_url: settings["textEndpoint"].as_str().unwrap_or_default().to_string(),
        default_model: settings["textModel"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("deepseek-chat")
            .to_string(),
        text_mode: settings["textMode"].as_str().unwrap_or("openai").to_string(),
        mode: String::new(),
        image_endpoint: String::new(),
        image_key: String::new(),
        image_model: String::new(),
        review_threshold: settings["reviewThreshold"].as_i64().unwrap_or(90) as u8,
        enable_local_save: settings["enableLocalSave"].as_bool().unwrap_or(false),
    };

    let active = screenplay_store::get_active_version(project_id, step_number);
    let current_output = active
        .as_ref()
        .and_then(|v| v.output.clone())
        .unwrap_or_else(|| "(无产出)".into());
    let current_selection = rec.selections.get(&step_number.to_string()).cloned();

    let params = server_proxy::ContextualLlmParams {
        runtime_config,
        context_type: "screenplay_selfcheck".into(),
        context_params: serde_json::json!({
            "stepNumber": step_number,
            "init": rec.init,
            "currentOutput": current_output,
            "currentSelection": current_selection,
        }),
        temperature: None,
        max_tokens_override: None,
    };

    let mut full_text = String::new();
    server_proxy::request_contextual_llm_stream(params, |chunk| {
        full_text.push_str(chunk);
        on_chunk(chunk);
    })
    .await
    .map_err(|e| e.to_string())?;

    let items = parse_selfcheck(&full_text);
    screenplay_store::save_selfcheck(project_id, step_number, items.clone());

    Ok(serde_json::json!({ "items": items }))
}

fn parse_step_output(step_number: u8, text: &str) -> Option<serde_json::Value> {
    step_parser::parse_step_output(step_number, text)
}

fn parse_selfcheck(text: &str) -> Vec<serde_json::Value> {
    step_parser::parse_selfcheck(text)
}

// ── Checkpoint ──

pub async fn generate_checkpoint(
    conn: &Connection,
    project_id: &str,
    trigger: &str,
) -> Result<String, String> {
    let rec = screenplay_store::load_project(project_id).ok_or("Project not found")?;

    let settings = crud::get_app_settings(conn);

    let runtime_config = RuntimeConfig {
        api_key: settings.text_key,
        api_base_url: settings.text_endpoint,
        default_model: settings.text_model,
        text_mode: settings.text_mode,
        mode: String::new(),
        image_endpoint: String::new(),
        image_key: String::new(),
        image_model: String::new(),
        review_threshold: 90,
        enable_local_save: true,
    };

    let project_snapshot = screenplay_store::build_project_snapshot(project_id);

    let params = server_proxy::ContextualLlmParams {
        runtime_config,
        context_type: "screenplay_checkpoint".into(),
        context_params: serde_json::json!({
            "init": rec.init,
            "projectSnapshot": project_snapshot,
        }),
        temperature: None,
        max_tokens_override: None,
    };

    let mut full_text = String::new();
    server_proxy::request_contextual_llm_stream(params, |chunk| {
        full_text.push_str(chunk);
    })
    .await
    .map_err(|e| e.to_string())?;

    let content = full_text.trim().to_string();
    if content.is_empty() {
        return Err("LLM 返回空, checkpoint 未生成".into());
    }

    screenplay_store::save_checkpoint(project_id, trigger, &content);
    Ok(content)
}

pub async fn generate_checkpoint_async(
    settings: serde_json::Value,
    project_id: &str,
    trigger: &str,
) -> Result<String, String> {
    let rec = screenplay_store::load_project(project_id).ok_or("Project not found")?;

    let runtime_config = RuntimeConfig {
        api_key: settings["textKey"].as_str().unwrap_or_default().to_string(),
        api_base_url: settings["textEndpoint"].as_str().unwrap_or_default().to_string(),
        default_model: settings["textModel"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("deepseek-chat")
            .to_string(),
        text_mode: settings["textMode"].as_str().unwrap_or("openai").to_string(),
        mode: String::new(),
        image_endpoint: String::new(),
        image_key: String::new(),
        image_model: String::new(),
        review_threshold: settings["reviewThreshold"].as_i64().unwrap_or(90) as u8,
        enable_local_save: settings["enableLocalSave"].as_bool().unwrap_or(false),
    };

    let project_snapshot = screenplay_store::build_project_snapshot(project_id);

    let params = server_proxy::ContextualLlmParams {
        runtime_config,
        context_type: "screenplay_checkpoint".into(),
        context_params: serde_json::json!({
            "init": rec.init,
            "projectSnapshot": project_snapshot,
        }),
        temperature: None,
        max_tokens_override: None,
    };

    let mut full_text = String::new();
    server_proxy::request_contextual_llm_stream(params, |chunk| {
        full_text.push_str(chunk);
    })
    .await
    .map_err(|e| e.to_string())?;

    let content = full_text.trim().to_string();
    if content.is_empty() {
        return Err("LLM 返回空, checkpoint 未生成".into());
    }

    screenplay_store::save_checkpoint(project_id, trigger, &content);
    Ok(content)
}

pub fn get_checkpoint(project_id: &str, trigger: &str) -> Option<String> {
    screenplay_store::get_checkpoint(project_id, trigger)
}

pub fn get_cached_selfcheck(project_id: &str, step_number: u8) -> Option<serde_json::Value> {
    screenplay_store::get_selfcheck(project_id, step_number).map(|s| {
        serde_json::json!({ "items": s.items, "createdAt": s.created_at })
    })
}

pub fn approve_step(
    project_id: &str,
    step_number: u8,
    next_step: Option<u8>,
) -> screenplay_store::ProjectRecord {
    screenplay_store::approve_step(project_id, step_number, next_step)
}

pub fn rollback_to(
    project_id: &str,
    target_step: u8,
) -> screenplay_store::ProjectRecord {
    screenplay_store::rollback_to(project_id, target_step)
}

pub fn list_versions(project_id: &str, step_number: u8) -> Vec<screenplay_store::VersionEntry> {
    screenplay_store::list_versions(project_id, step_number)
}

pub fn restore_version(
    project_id: &str,
    step_number: u8,
    version_id: &str,
) {
    screenplay_store::set_active_version(project_id, step_number, version_id);
}

pub fn set_step_selection(project_id: &str, step_number: u8, selection_id: Option<String>) {
    screenplay_store::set_step_selection(project_id, step_number, selection_id);
}
