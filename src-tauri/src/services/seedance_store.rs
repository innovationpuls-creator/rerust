use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::utils::v5_parser::{V5Analysis, NoteArea};

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn uuid() -> String {
    Uuid::new_v4().to_string()
}

/// Save (upsert) V5 analysis to seedance_analysis table.
pub fn save_analysis(conn: &Connection, task_id: &str, analysis: &V5Analysis) {
    let t = now();
    let p_idx = serde_json::to_string(&analysis.paragraph_facts).unwrap_or_default();
    let st = &analysis.structure_type;
    let em = serde_json::to_string(&analysis.emotion_map).unwrap_or_default();
    let up = serde_json::to_string(&analysis.units).unwrap_or_default();
    let ts = analysis.total_sec as i64;
    let tu = analysis.total_units as i64;
    conn.execute(
        "INSERT INTO seedance_analysis (task_id, paragraph_index_json, structure_type, emotion_map_json, units_plan_json, total_sec, total_units, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(task_id) DO UPDATE SET
           paragraph_index_json = excluded.paragraph_index_json,
           structure_type = excluded.structure_type,
           emotion_map_json = excluded.emotion_map_json,
           units_plan_json = excluded.units_plan_json,
           total_sec = excluded.total_sec,
           total_units = excluded.total_units,
           updated_at = excluded.updated_at",
        params![task_id, &p_idx, st, &em, &up, ts, tu, t, t],
    )
    .ok();
}

/// Load V5 analysis from seedance_analysis table.
pub fn load_analysis(conn: &Connection, task_id: &str) -> Option<V5Analysis> {
    let row: Result<(String, String, String, String, i64, i64), _> = conn.query_row(
        "SELECT paragraph_index_json, structure_type, emotion_map_json, units_plan_json, total_sec, total_units FROM seedance_analysis WHERE task_id = ?1",
        params![task_id],
        |row| Ok((
            row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?,
        )),
    );
    let (p_idx, st, em, up, ts, tu) = row.ok()?;
    let paragraph_facts: Vec<crate::utils::v5_parser::ParagraphFact> =
        serde_json::from_str(&p_idx).unwrap_or_default();
    let emotion_map: crate::utils::v5_parser::EmotionMap =
        serde_json::from_str(&em).unwrap_or_default();
    let units: Vec<crate::utils::v5_parser::V5UnitPlan> =
        serde_json::from_str(&up).unwrap_or_default();
    Some(V5Analysis {
        paragraph_facts,
        structure_type: st,
        emotion_map,
        units,
        total_sec: ts as usize,
        total_units: tu as usize,
        warnings: vec![],
    })
}

/// Delete all analysis data for a task.
pub fn delete_analysis(conn: &Connection, task_id: &str) {
    conn.execute("DELETE FROM seedance_analysis WHERE task_id = ?1", params![task_id])
        .ok();
}

/// Upsert a single unit record in seedance_units table.
pub fn upsert_unit(conn: &Connection, task_id: &str, unit_index: i32, duration_sec: Option<i32>, scene_type: &str, sub_shot_count: Option<i32>, copy_area: &str, note_area: &NoteArea, status: &str, retry_count: i32, error_message: Option<&str>) {
    let t = now();
    let note_json = serde_json::to_string(note_area).unwrap_or_default();
    conn.execute(
        "INSERT INTO seedance_units (id, task_id, unit_index, duration_sec, scene_type, sub_shot_count, copy_area, note_area_json, status, retry_count, error_message, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(task_id, unit_index) DO UPDATE SET
           duration_sec = excluded.duration_sec,
           scene_type = excluded.scene_type,
           sub_shot_count = excluded.sub_shot_count,
           copy_area = excluded.copy_area,
           note_area_json = excluded.note_area_json,
           status = excluded.status,
           retry_count = excluded.retry_count,
           error_message = excluded.error_message,
           updated_at = excluded.updated_at",
        params![uuid(), task_id, unit_index, duration_sec, scene_type, sub_shot_count, copy_area, &note_json, status, retry_count, error_message, t, t],
    )
    .ok();
}

/// List all units for a task, ordered by unit_index.
pub fn list_units(conn: &Connection, task_id: &str) -> Vec<serde_json::Value> {
    let mut stmt = conn
        .prepare("SELECT * FROM seedance_units WHERE task_id = ?1 ORDER BY unit_index")
        .unwrap();
    stmt.query_map(params![task_id], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, String>("id").ok(),
            "taskId": row.get::<_, String>("task_id").ok(),
            "unitIndex": row.get::<_, i32>("unit_index").ok(),
            "durationSec": row.get::<_, Option<i32>>("duration_sec").ok().flatten(),
            "sceneType": row.get::<_, Option<String>>("scene_type").ok().flatten(),
            "subShotCount": row.get::<_, Option<i32>>("sub_shot_count").ok().flatten(),
            "copyArea": row.get::<_, Option<String>>("copy_area").ok().flatten(),
            "noteAreaJson": row.get::<_, Option<String>>("note_area_json").ok().flatten(),
            "status": row.get::<_, String>("status").ok(),
            "retryCount": row.get::<_, i32>("retry_count").ok(),
            "errorMessage": row.get::<_, Option<String>>("error_message").ok().flatten(),
            "createdAt": row.get::<_, String>("created_at").ok(),
            "updatedAt": row.get::<_, String>("updated_at").ok(),
        }))
    })
    .unwrap()
    .filter_map(|r| r.ok())
    .collect()
}

/// Get a single unit record.
pub fn get_unit(conn: &Connection, task_id: &str, unit_index: i32) -> Option<serde_json::Value> {
    conn.query_row(
        "SELECT * FROM seedance_units WHERE task_id = ?1 AND unit_index = ?2",
        params![task_id, unit_index],
        |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>("id").ok(),
                "taskId": row.get::<_, String>("task_id").ok(),
                "unitIndex": row.get::<_, i32>("unit_index").ok(),
                "durationSec": row.get::<_, Option<i32>>("duration_sec").ok().flatten(),
                "sceneType": row.get::<_, Option<String>>("scene_type").ok().flatten(),
                "subShotCount": row.get::<_, Option<i32>>("sub_shot_count").ok().flatten(),
                "copyArea": row.get::<_, Option<String>>("copy_area").ok().flatten(),
                "noteAreaJson": row.get::<_, Option<String>>("note_area_json").ok().flatten(),
                "status": row.get::<_, String>("status").ok(),
                "retryCount": row.get::<_, i32>("retry_count").ok(),
                "errorMessage": row.get::<_, Option<String>>("error_message").ok().flatten(),
                "createdAt": row.get::<_, String>("created_at").ok(),
                "updatedAt": row.get::<_, String>("updated_at").ok(),
            }))
        },
    )
    .ok()
}

/// Delete all units for a task.
pub fn delete_units(conn: &Connection, task_id: &str) {
    conn.execute("DELETE FROM seedance_units WHERE task_id = ?1", params![task_id])
        .ok();
}

/// Delete a single unit.
pub fn delete_unit(conn: &Connection, task_id: &str, unit_index: i32) {
    conn.execute(
        "DELETE FROM seedance_units WHERE task_id = ?1 AND unit_index = ?2",
        params![task_id, unit_index],
    )
    .ok();
}
