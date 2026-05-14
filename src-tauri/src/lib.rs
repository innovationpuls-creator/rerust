pub mod db;
pub mod llm;
pub mod services;
pub mod utils;

use std::sync::Mutex;
use tauri::Manager;

mod cmd {
    use crate::db::crud;
    use crate::services::screenplay;
    use crate::services::screenplay_store;
    use rusqlite::Connection;
    use std::sync::Mutex;
    use tauri::{AppHandle, Emitter, State};
    use sha2::{Digest, Sha256};

    fn with_db<F, R>(state: &State<'_, Mutex<Connection>>, f: F) -> Result<R, String>
    where
        F: FnOnce(&Connection) -> R,
    {
        let c = state.lock().map_err(|e| e.to_string())?;
        Ok(f(&c))
    }

    #[tauri::command]
    pub fn get_version() -> String {
        "2.0.5".to_string()
    }

    #[tauri::command]
    pub fn get_database_meta(state: State<'_, Mutex<Connection>>) -> serde_json::Value {
        with_db(&state, |c| {
            let path: String = c
                .query_row("PRAGMA database_list", [], |row| row.get::<_, String>(2))
                .unwrap_or_default();
            let p = std::path::Path::new(&path);
            let data_dir = p
                .parent()
                .map(|d| d.to_string_lossy().to_string())
                .unwrap_or_default();
            serde_json::json!({ "dbPath": path, "dataDir": data_dir })
        })
        .unwrap_or_else(|e| serde_json::json!({ "error": e }))
    }

    #[tauri::command]
    pub fn get_app_settings(state: State<'_, Mutex<Connection>>) -> Result<crud::AppSettings, String> {
        with_db(&state, crud::get_app_settings)
    }

    #[tauri::command]
    pub fn save_app_settings(
        state: State<'_, Mutex<Connection>>,
        payload: crud::AppSettings,
    ) -> Result<crud::AppSettings, String> {
        with_db(&state, |c| crud::save_app_settings(c, &payload))
    }

    #[tauri::command]
    pub async fn test_connection(payload: serde_json::Value) -> serde_json::Value {
        let endpoint = payload["endpoint"].as_str().unwrap_or("");
        let key = payload["key"].as_str().unwrap_or("");
        let model = payload["model"].as_str().unwrap_or("");
        let test_type = payload["type"].as_str().unwrap_or("text");
        let mode = payload["mode"].as_str().unwrap_or("openai");

        if test_type == "image" {
            // Image endpoints use openai-compatible format
            let start = std::time::Instant::now();
            let url = format!("{}/images/generations", endpoint.trim_end_matches('/'));
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default();
            let response = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", key))
                .json(&serde_json::json!({
                    "model": model,
                    "prompt": "test",
                    "n": 1,
                    "size": "256x256"
                }))
                .send()
                .await;
            return match response {
                Ok(r) => {
                    let latency = start.elapsed().as_millis() as u64;
                    let status = r.status();
                    if status.is_success() {
                        serde_json::json!({ "ok": true, "latencyMs": latency })
                    } else {
                        let code = status.as_u16();
                        let detail = r.text().await.unwrap_or_default();
                        serde_json::json!({ "ok": false, "latencyMs": latency, "error": format!("HTTP {}: {}", code, &detail[..detail.len().min(200)]) })
                    }
                }
                Err(e) => serde_json::json!({ "ok": false, "latencyMs": 0, "error": format!("网络错误：{}", e) }),
            };
        }

        match crate::llm::server_proxy::test_connection(endpoint, key, model, mode).await {
            Ok((ok, latency, err)) => {
                if ok {
                    serde_json::json!({ "ok": true, "latencyMs": latency })
                } else {
                    serde_json::json!({ "ok": false, "latencyMs": latency, "error": err })
                }
            }
            Err(e) => serde_json::json!({ "ok": false, "latencyMs": 0, "error": e }),
        }
    }

    #[tauri::command]
    pub fn get_recent_script_tasks(
        state: State<'_, Mutex<Connection>>,
    ) -> Result<Vec<crud::ScriptTaskSummary>, String> {
        with_db(&state, |c| crud::get_recent_script_tasks(c, 8))
    }

    #[tauri::command]
    pub fn get_recent_image_tasks(
        state: State<'_, Mutex<Connection>>,
    ) -> Result<Vec<serde_json::Value>, String> {
        with_db(&state, |c| crud::get_recent_image_tasks(c, 8))
    }

    #[tauri::command]
    pub fn get_recent_video_tasks(
        state: State<'_, Mutex<Connection>>,
    ) -> Result<Vec<serde_json::Value>, String> {
        with_db(&state, |c| crud::get_recent_video_tasks(c, 8))
    }

    #[tauri::command]
    pub fn load_script_task(
        state: State<'_, Mutex<Connection>>,
        task_id: String,
    ) -> Result<Option<serde_json::Value>, String> {
        with_db(&state, |c| crud::load_script_task(c, &task_id))
    }

    #[tauri::command]
    pub fn delete_script_task(state: State<'_, Mutex<Connection>>, task_id: String) -> serde_json::Value {
        with_db(&state, |c| crud::delete_script_task(c, &task_id)).ok();
        serde_json::json!({ "success": true, "taskId": task_id })
    }

    #[tauri::command]
    pub fn delete_image_task(state: State<'_, Mutex<Connection>>, task_id: String) -> serde_json::Value {
        with_db(&state, |c| crud::delete_image_task(c, &task_id)).ok();
        serde_json::json!({ "success": true, "taskId": task_id })
    }

    #[tauri::command]
    pub fn delete_video_task(state: State<'_, Mutex<Connection>>, task_id: String) -> serde_json::Value {
        with_db(&state, |c| crud::delete_video_task(c, &task_id)).ok();
        serde_json::json!({ "success": true, "taskId": task_id })
    }

    #[tauri::command]
    pub fn save_script_draft(
        state: State<'_, Mutex<Connection>>,
        payload: crud::ScriptDraftInput,
    ) -> Result<crud::ScriptDraftResult, String> {
        with_db(&state, |c| crud::save_script_draft(c, &payload))
    }

    #[tauri::command]
    pub fn save_image_prompt_draft(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        with_db(&state, |c| {
            let input = crud::ImageVideoDraftInput {
                mode: payload["mode"].as_str().unwrap_or("").to_string(),
                source_script: payload["sourceScript"].as_str().map(String::from),
                visual_style: payload["visualStyle"].as_str().map(String::from),
                image_goal: payload["imageGoal"].as_str().map(String::from),
                project_id: payload["projectId"].as_str().map(String::from),
                script_beats: None,
                video_style: None,
                motion_focus: None,
            };
            let r = crud::save_image_draft(c, &input);
            serde_json::json!({ "projectId": r.project_id, "taskId": r.task_id, "savedAt": r.saved_at })
        })
    }

    #[tauri::command]
    pub fn save_video_prompt_draft(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        with_db(&state, |c| {
            let input = crud::ImageVideoDraftInput {
                mode: payload["mode"].as_str().unwrap_or("").to_string(),
                project_id: payload["projectId"].as_str().map(String::from),
                script_beats: payload["scriptBeats"].as_str().map(String::from),
                video_style: payload["videoStyle"].as_str().map(String::from),
                motion_focus: payload["motionFocus"].as_str().map(String::from),
                source_script: None,
                visual_style: None,
                image_goal: None,
            };
            let r = crud::save_video_draft(c, &input);
            serde_json::json!({ "projectId": r.project_id, "taskId": r.task_id, "savedAt": r.saved_at })
        })
    }

    #[tauri::command]
    pub fn save_script_generation(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        with_db(&state, |c| {
            let input = crud::ScriptGenerationInput {
                mode: payload["mode"].as_str().unwrap_or("plot").to_string(),
                duration: payload["duration"].as_str().map(String::from),
                input_summary: payload["inputSummary"].as_str().unwrap_or("").to_string(),
                style_preset: payload["stylePreset"].as_str().map(String::from),
                genres: payload["genres"].as_str().map(String::from),
                audience: payload["audience"].as_str().map(String::from),
                tone: payload["tone"].as_str().map(String::from),
                ending: payload["ending"].as_str().map(String::from),
                output_mode: payload["outputMode"].as_str().map(String::from),
                episodes: payload["episodes"].as_str().map(String::from),
                custom_style: payload["customStyle"].as_str().map(String::from),
                existing_project_id: payload["existingProjectId"].as_str().map(String::from),
                existing_task_id: payload["existingTaskId"].as_str().map(String::from),
            };
            let sections = crud::fallback_sections(&input.mode);
            let result = crud::save_script_generation(c, &input, sections, vec![], None);
            serde_json::to_value(&result).unwrap_or_default()
        })
    }

    #[tauri::command]
    pub fn update_script_body(state: State<'_, Mutex<Connection>>, task_id: String, new_body: String) {
        with_db(&state, |c| crud::update_script_body(c, &task_id, &new_body)).ok();
    }

    #[tauri::command]
    pub fn import_existing_script(
        state: State<'_, Mutex<Connection>>,
        payload: crud::ImportScriptInput,
    ) -> Result<crud::ScriptGenerationResult, String> {
        with_db(&state, |c| crud::import_existing_script(c, &payload))
    }

    #[tauri::command]
    pub fn run_script_review(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        with_db(&state, |c| crud::run_script_review(c, &payload))?
    }

    #[tauri::command]
    pub fn run_image_generation(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        with_db(&state, |c| crud::run_image_generation(c, &payload))
    }

    #[tauri::command]
    pub fn run_video_generation(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        with_db(&state, |c| crud::run_video_generation(c, &payload))
    }

    #[tauri::command]
    pub fn run_image_review(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        with_db(&state, |c| crud::run_image_review(c, &payload))
    }

    #[tauri::command]
    pub fn run_video_review(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        with_db(&state, |c| crud::run_video_review(c, &payload))
    }

    #[tauri::command]
    pub fn run_asset_extraction(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let conn = state.lock().map_err(|e| e.to_string())?;
        crud::run_asset_extraction(&conn, &payload)
    }

    #[tauri::command]
    pub fn get_assets_by_task(
        state: State<'_, Mutex<Connection>>,
        task_id: String,
    ) -> Result<Vec<serde_json::Value>, String> {
        with_db(&state, |c| crud::get_assets_by_task(c, &task_id))
    }

    #[tauri::command]
    pub fn update_assets(
        state: State<'_, Mutex<Connection>>,
        task_id: String,
        characters: String,
        scenes: String,
        props: String,
    ) {
        with_db(&state, |c| crud::update_assets(c, &task_id, &characters, &scenes, &props)).ok();
    }

    #[tauri::command]
    pub fn run_prompt_generation(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let conn = state.lock().map_err(|e| e.to_string())?;
        crud::run_prompt_generation(&conn, &payload)
    }

    #[tauri::command]
    pub fn run_prompt_group_generation(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let conn = state.lock().map_err(|e| e.to_string())?;
        crud::run_prompt_group_gen(&conn, &payload)
    }

    #[tauri::command]
    pub fn update_prompt_output(state: State<'_, Mutex<Connection>>, task_id: String, seedance_groups: String) {
        with_db(&state, |c| crud::update_prompt_output(c, &task_id, &seedance_groups)).ok();
    }

    #[tauri::command]
    pub fn get_prompt_output_by_task(
        state: State<'_, Mutex<Connection>>,
        task_id: String,
    ) -> Result<Option<serde_json::Value>, String> {
        with_db(&state, |c| crud::get_prompt_output_by_task(c, &task_id))
    }

    #[tauri::command]
    pub fn get_scene_count(
        state: State<'_, Mutex<Connection>>,
        task_id: String,
    ) -> Result<Option<i64>, String> {
        with_db(&state, |c| crud::get_scene_count(c, &task_id))
    }

    #[tauri::command]
    pub fn get_segment_titles(
        state: State<'_, Mutex<Connection>>,
        task_id: String,
    ) -> Result<Vec<String>, String> {
        with_db(&state, |c| crud::get_segment_titles(c, &task_id))
    }

    #[tauri::command]
    pub fn run_prompt_quality_check(
        state: State<'_, Mutex<Connection>>,
        task_id: String,
    ) -> Result<serde_json::Value, String> {
        with_db(&state, |c| crud::run_quality_check(c, &task_id))
    }

    #[tauri::command]
    pub fn generate_outline(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let conn = state.lock().map_err(|e| e.to_string())?;
        crud::generate_outline(&conn, &payload)
    }

    #[tauri::command]
    pub fn confirm_outline(state: State<'_, Mutex<Connection>>, payload: serde_json::Value) {
        with_db(&state, |c| crud::confirm_outline(c, &payload)).ok();
    }

    #[tauri::command]
    pub fn get_outline(
        state: State<'_, Mutex<Connection>>,
        task_id: String,
    ) -> Result<Option<serde_json::Value>, String> {
        with_db(&state, |c| crud::get_outline(c, &task_id))
    }

    // ── Projects ──

    #[tauri::command]
    pub fn get_projects(
        state: State<'_, Mutex<Connection>>,
    ) -> Result<Vec<serde_json::Value>, String> {
        with_db(&state, crud::get_projects)
    }

    #[tauri::command]
    pub fn rename_project(
        state: State<'_, Mutex<Connection>>,
        project_id: String,
        new_name: String,
    ) -> Result<serde_json::Value, String> {
        let _ = with_db(&state, |c| crud::rename_project(c, &project_id, &new_name));
        Ok(serde_json::json!({ "success": true }))
    }

    #[tauri::command]
    pub fn delete_project(
        state: State<'_, Mutex<Connection>>,
        project_id: String,
    ) -> Result<serde_json::Value, String> {
        let _ = with_db(&state, |c| crud::delete_project(c, &project_id));
        Ok(serde_json::json!({ "success": true, "projectId": project_id }))
    }

    // ── Seedance ──

    #[tauri::command]
    pub fn seedance_run_phase_ad(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let task_id = payload["taskId"].as_str().unwrap_or("").to_string();
        let conn = state.lock().map_err(|e| e.to_string())?;
        crud::seedance_phase_ad(&conn, &task_id)
    }

    #[tauri::command]
    pub fn seedance_get_analysis(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<Option<serde_json::Value>, String> {
        let task_id = payload["taskId"].as_str().unwrap_or("");
        with_db(&state, |c| crud::seedance_get_analysis(c, task_id))
    }

    #[tauri::command]
    pub fn seedance_run_unit(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let task_id = payload["taskId"].as_str().unwrap_or("").to_string();
        let unit_index = payload["unitIndex"].as_i64().unwrap_or(0) as i32;
        let conn = state.lock().map_err(|e| e.to_string())?;
        crud::seedance_run_unit(&conn, &task_id, unit_index)
    }

    #[tauri::command]
    pub fn seedance_run_all(
        app: AppHandle,
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let task_id = payload["taskId"].as_str().unwrap_or("").to_string();
        let conn = state.lock().map_err(|e| e.to_string())?;
        let result = crud::seedance_run_all(&conn, &task_id)?;
        app.emit("seedance:progress", serde_json::json!({ "taskId": task_id, "progress": 1.0 }))
            .ok();
        Ok(result)
    }

    #[tauri::command]
    pub fn seedance_list_units(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, String> {
        let task_id = payload["taskId"].as_str().unwrap_or("");
        with_db(&state, |c| crud::seedance_list_units(c, task_id))
    }

    #[tauri::command]
    pub fn seedance_get_unit(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<Option<serde_json::Value>, String> {
        let task_id = payload["taskId"].as_str().unwrap_or("");
        let unit_index = payload["unitIndex"].as_i64().unwrap_or(0) as i32;
        with_db(&state, |c| crud::seedance_get_unit(c, task_id, unit_index))
    }

    // ── Auth / Token ──

    #[tauri::command]
    pub fn set_auth_token(_app: AppHandle, token: String, _refresh_token: String) {
        std::env::set_var("CINEFORGE_AUTH_TOKEN", &token);
    }

    fn hash_password(password: &str, salt: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(salt.as_bytes());
        hasher.update(password.as_bytes());
        hex::encode(hasher.finalize())
    }

    #[tauri::command]
    pub fn auth_login(
        state: State<'_, Mutex<Connection>>,
        username: String,
        password: String,
    ) -> serde_json::Value {
        let result = with_db(&state, |c| {
            let mut stmt = c
                .prepare("SELECT password_hash, salt, email FROM users WHERE username = ?1")
                .ok()?;
            let row = stmt.query_row([&username], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .ok()?;
            Some(row)
        });

        match result {
            Ok(Some((hash, salt, _email))) => {
                if hash_password(&password, &salt) != hash {
                    return serde_json::json!({ "error": "用户名或密码错误" });
                }
                let token = uuid::Uuid::new_v4().to_string();
                let refresh_token = uuid::Uuid::new_v4().to_string();
                let _ = with_db(&state, |c| {
                    c.execute(
                        "UPDATE users SET token = ?1, refresh_token = ?2 WHERE username = ?3",
                        rusqlite::params![token, refresh_token, username],
                    )
                });
                serde_json::json!({ "token": token, "refreshToken": refresh_token })
            }
            _ => serde_json::json!({ "error": "用户名或密码错误" }),
        }
    }

    #[tauri::command]
    pub fn auth_register(
        state: State<'_, Mutex<Connection>>,
        username: String,
        password: String,
        email: String,
    ) -> serde_json::Value {
        // Check if user exists
        let exists = with_db(&state, |c| {
            c.query_row(
                "SELECT COUNT(*) FROM users WHERE username = ?1",
                [&username],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
        })
        .unwrap_or(0);

        if exists > 0 {
            return serde_json::json!({ "error": "用户名已被注册" });
        }

        let salt = uuid::Uuid::new_v4().to_string();
        let hash = hash_password(&password, &salt);
        let token = uuid::Uuid::new_v4().to_string();
        let refresh_token = uuid::Uuid::new_v4().to_string();

        let ok = with_db(&state, |c| {
            c.execute(
                "INSERT INTO users (username, email, password_hash, salt, token, refresh_token) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![username, email, hash, salt, token, refresh_token],
            )
        });

        match ok {
            Ok(_) => serde_json::json!({ "token": token, "refreshToken": refresh_token }),
            Err(e) => serde_json::json!({ "error": format!("注册失败: {}", e) }),
        }
    }

    #[tauri::command]
    pub fn auth_refresh(
        state: State<'_, Mutex<Connection>>,
        refresh_token: String,
    ) -> serde_json::Value {
        let result = with_db(&state, |c| {
            let mut stmt = c
                .prepare("SELECT username FROM users WHERE refresh_token = ?1")
                .ok()?;
            let username = stmt
                .query_row([&refresh_token], |r| r.get::<_, String>(0))
                .ok()?;
            Some(username)
        });

        match result {
            Ok(Some(username)) => {
                let new_token = uuid::Uuid::new_v4().to_string();
                let _ = with_db(&state, |c| {
                    c.execute(
                        "UPDATE users SET token = ?1 WHERE username = ?2",
                        rusqlite::params![new_token, username],
                    )
                });
                serde_json::json!({ "token": new_token })
            }
            _ => serde_json::json!({ "error": "refresh token 无效" }),
        }
    }

    // ── Screenplay ──

    #[tauri::command]
    pub fn screenplay_skill_status() -> serde_json::Value {
        screenplay::skill_status()
    }

    #[tauri::command]
    pub fn screenplay_create_project(
        init: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let project_init = screenplay_store::ProjectInit {
            name: init["name"].as_str().map(String::from),
            concept: init["concept"].as_str().map(String::from),
            duration: init["duration"].as_str().map(String::from),
            path: init["path"].as_str().map(String::from),
            imported_script: init["importedScript"].as_str().map(String::from),
            imported_file_name: init["importedFileName"].as_str().map(String::from),
            format: init["format"].as_str().map(String::from),
            ultrashort_mode: init["ultrashortMode"].as_str().map(String::from),
            genres: init["genres"].as_array().map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            }),
            chinese: init["chinese"].as_bool(),
            master: init["master"].as_str().map(String::from),
        };
        let rec = screenplay::create_project(project_init);
        serde_json::to_value(&rec).map_err(|e| e.to_string())
    }

    #[tauri::command]
    pub fn screenplay_get_project(
        project_id: String,
    ) -> Result<Option<serde_json::Value>, String> {
        Ok(screenplay::get_project(&project_id).and_then(|r| serde_json::to_value(r).ok()))
    }

    #[tauri::command]
    pub fn screenplay_list_recent_projects(
        limit: Option<usize>,
    ) -> Vec<serde_json::Value> {
        screenplay::list_recent_projects(limit.unwrap_or(20))
    }

    #[tauri::command]
    pub fn screenplay_delete_project(project_id: String) -> serde_json::Value {
        let ok = screenplay::delete_project(&project_id);
        serde_json::json!({ "success": ok })
    }

    #[tauri::command]
    pub fn screenplay_update_step_structured(
        payload: serde_json::Value,
    ) -> serde_json::Value {
        let pid = payload["projectId"].as_str().unwrap_or("");
        let step = payload["stepNumber"].as_i64().unwrap_or(1) as u8;
        let structured = payload["structured"].clone();
        let ok = screenplay::update_step_structured(pid, step, structured);
        serde_json::json!({ "success": ok })
    }

    #[tauri::command]
    pub fn screenplay_rename_project(payload: serde_json::Value) -> serde_json::Value {
        let pid = payload["projectId"].as_str().unwrap_or("");
        let name = payload["newName"].as_str().unwrap_or("");
        let ok = screenplay::rename_project(pid, name);
        serde_json::json!({ "success": ok })
    }

    #[tauri::command]
    pub fn screenplay_finalize_to_script_task(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let pid = payload["projectId"].as_str().unwrap_or("");
        let conn = state.lock().map_err(|e| e.to_string())?;
        screenplay::finalize_to_script_task(&conn, pid)
    }

    #[tauri::command]
    pub async fn screenplay_generate_step(
        app: AppHandle,
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let pid = payload["projectId"].as_str().unwrap_or("").to_string();
        let step = payload["stepNumber"].as_i64().unwrap_or(1) as u8;
        let feedback = payload["userFeedback"].as_str().map(String::from);

        let settings = {
            let conn = state.lock().map_err(|e| e.to_string())?;
            crud::get_app_settings(&conn)
        };
        let settings_json = serde_json::to_value(&settings).unwrap_or_default();

        screenplay::generate_step_async(
            settings_json, &pid, step, feedback,
            |chunk| {
                app.emit(
                    "screenplay:stream-chunk",
                    serde_json::json!({ "projectId": pid, "stepNumber": step, "chunk": chunk }),
                )
                .ok();
            },
        )
        .await
    }

    #[tauri::command]
    pub async fn screenplay_selfcheck_step(
        app: AppHandle,
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let pid = payload["projectId"].as_str().unwrap_or("").to_string();
        let step = payload["stepNumber"].as_i64().unwrap_or(1) as u8;

        let settings = {
            let conn = state.lock().map_err(|e| e.to_string())?;
            crud::get_app_settings(&conn)
        };
        let settings_json = serde_json::to_value(&settings).unwrap_or_default();

        screenplay::selfcheck_step_async(
            settings_json, &pid, step,
            |chunk| {
                app.emit(
                    "screenplay:stream-chunk",
                    serde_json::json!({ "projectId": pid, "stepNumber": step, "chunk": chunk }),
                )
                .ok();
            },
        )
        .await
    }

    #[tauri::command]
    pub fn screenplay_get_cached_selfcheck(payload: serde_json::Value) -> Option<serde_json::Value> {
        let pid = payload["projectId"].as_str().unwrap_or("");
        let step = payload["stepNumber"].as_i64().unwrap_or(1) as u8;
        screenplay::get_cached_selfcheck(pid, step)
    }

    #[tauri::command]
    pub fn screenplay_approve_step(payload: serde_json::Value) -> serde_json::Value {
        let pid = payload["projectId"].as_str().unwrap_or("");
        let step = payload["stepNumber"].as_i64().unwrap_or(1) as u8;
        let next = payload["nextStep"].as_i64().map(|n| n as u8);
        let rec = screenplay::approve_step(pid, step, next);
        serde_json::to_value(&rec).unwrap_or_default()
    }

    #[tauri::command]
    pub fn screenplay_rollback_to(payload: serde_json::Value) -> serde_json::Value {
        let pid = payload["projectId"].as_str().unwrap_or("");
        let target = payload["targetStep"].as_i64().unwrap_or(1) as u8;
        let rec = screenplay::rollback_to(pid, target);
        serde_json::to_value(&rec).unwrap_or_default()
    }

    #[tauri::command]
    pub fn screenplay_list_versions(payload: serde_json::Value) -> Vec<serde_json::Value> {
        let pid = payload["projectId"].as_str().unwrap_or("");
        let step = payload["stepNumber"].as_i64().unwrap_or(1) as u8;
        screenplay::list_versions(pid, step)
            .into_iter()
            .filter_map(|v| serde_json::to_value(&v).ok())
            .collect()
    }

    #[tauri::command]
    pub fn screenplay_restore_version(payload: serde_json::Value) {
        let pid = payload["projectId"].as_str().unwrap_or("");
        let step = payload["stepNumber"].as_i64().unwrap_or(1) as u8;
        let vid = payload["versionId"].as_str().unwrap_or("");
        screenplay::restore_version(pid, step, vid);
    }

    #[tauri::command]
    pub fn screenplay_set_step_selection(payload: serde_json::Value) {
        let pid = payload["projectId"].as_str().unwrap_or("");
        let step = payload["stepNumber"].as_i64().unwrap_or(1) as u8;
        let sel = payload["selectionId"].as_str().map(String::from);
        screenplay::set_step_selection(pid, step, sel);
    }

    #[tauri::command]
    pub fn screenplay_get_checkpoint(payload: serde_json::Value) -> Option<String> {
        let pid = payload["projectId"].as_str().unwrap_or("");
        let trigger = payload["trigger"].as_str().unwrap_or("");
        screenplay::get_checkpoint(pid, trigger)
    }

    #[tauri::command]
    pub async fn screenplay_regenerate_checkpoint(
        state: State<'_, Mutex<Connection>>,
        payload: serde_json::Value,
    ) -> Result<String, String> {
        let pid = payload["projectId"].as_str().unwrap_or("").to_string();
        let trigger = payload["trigger"].as_str().unwrap_or("after-step-6").to_string();

        let settings = {
            let conn = state.lock().map_err(|e| e.to_string())?;
            crud::get_app_settings(&conn)
        };
        let settings_json = serde_json::to_value(&settings).unwrap_or_default();

        screenplay::generate_checkpoint_async(settings_json, &pid, &trigger).await
    }

    // ── File dialogs ──

    #[tauri::command]
    pub fn select_text_file() -> serde_json::Value {
        serde_json::json!({ "cancelled": true, "content": "" })
    }

    #[tauri::command]
    pub fn select_image_file() -> serde_json::Value {
        serde_json::json!({ "cancelled": true, "base64": "", "mimeType": "" })
    }
}

pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let db_path = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir")
                .join("cineforge.db");

            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent).ok();
            }

            let conn = db::init_database(&db_path).expect("failed to initialize database");
            app.manage(Mutex::new(conn));

            // Initialize screenplay projects directory
            let app_data = app.path().app_data_dir().expect("failed to resolve app data dir");
            services::screenplay_store::init_projects_dir(&app_data);

            log::info!("CineForge started, database at: {:?}", db_path);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            cmd::get_version,
            cmd::get_database_meta,
            cmd::get_app_settings,
            cmd::save_app_settings,
            cmd::test_connection,
            cmd::get_recent_script_tasks,
            cmd::get_recent_image_tasks,
            cmd::get_recent_video_tasks,
            cmd::load_script_task,
            cmd::delete_script_task,
            cmd::delete_image_task,
            cmd::delete_video_task,
            cmd::save_script_draft,
            cmd::save_image_prompt_draft,
            cmd::save_video_prompt_draft,
            cmd::save_script_generation,
            cmd::update_script_body,
            cmd::import_existing_script,
            cmd::run_script_review,
            cmd::run_image_generation,
            cmd::run_video_generation,
            cmd::run_image_review,
            cmd::run_video_review,
            cmd::run_asset_extraction,
            cmd::get_assets_by_task,
            cmd::update_assets,
            cmd::run_prompt_generation,
            cmd::run_prompt_group_generation,
            cmd::update_prompt_output,
            cmd::get_prompt_output_by_task,
            cmd::get_scene_count,
            cmd::get_segment_titles,
            cmd::run_prompt_quality_check,
            cmd::generate_outline,
            cmd::confirm_outline,
            cmd::get_outline,
            cmd::get_projects,
            cmd::rename_project,
            cmd::delete_project,
            cmd::seedance_run_phase_ad,
            cmd::seedance_get_analysis,
            cmd::seedance_run_unit,
            cmd::seedance_run_all,
            cmd::seedance_list_units,
            cmd::seedance_get_unit,
            cmd::set_auth_token,
            cmd::auth_login,
            cmd::auth_register,
            cmd::auth_refresh,
            cmd::screenplay_skill_status,
            cmd::screenplay_create_project,
            cmd::screenplay_get_project,
            cmd::screenplay_list_recent_projects,
            cmd::screenplay_delete_project,
            cmd::screenplay_update_step_structured,
            cmd::screenplay_rename_project,
            cmd::screenplay_finalize_to_script_task,
            cmd::screenplay_generate_step,
            cmd::screenplay_selfcheck_step,
            cmd::screenplay_get_cached_selfcheck,
            cmd::screenplay_approve_step,
            cmd::screenplay_rollback_to,
            cmd::screenplay_list_versions,
            cmd::screenplay_restore_version,
            cmd::screenplay_set_step_selection,
            cmd::screenplay_get_checkpoint,
            cmd::screenplay_regenerate_checkpoint,
            cmd::select_text_file,
            cmd::select_image_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running CineForge");
}
