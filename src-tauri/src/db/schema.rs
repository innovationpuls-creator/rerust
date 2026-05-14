use rusqlite::Connection;

pub fn create_tables(conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS projects (
          id TEXT PRIMARY KEY,
          name TEXT NOT NULL,
          module_type TEXT NOT NULL,
          status TEXT NOT NULL DEFAULT 'draft',
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS script_tasks (
          id TEXT PRIMARY KEY,
          project_id TEXT NOT NULL,
          mode TEXT NOT NULL,
          input_summary TEXT,
          genre TEXT,
          style TEXT,
          duration TEXT,
          stage TEXT NOT NULL DEFAULT 'idle',
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          FOREIGN KEY (project_id) REFERENCES projects(id)
        );

        CREATE TABLE IF NOT EXISTS script_outputs (
          id TEXT PRIMARY KEY,
          task_id TEXT NOT NULL,
          characters_json TEXT,
          plot_outline TEXT,
          script_body TEXT,
          hook_opening TEXT,
          storyboard_base TEXT,
          raw_response TEXT,
          created_at TEXT NOT NULL,
          FOREIGN KEY (task_id) REFERENCES script_tasks(id)
        );

        CREATE TABLE IF NOT EXISTS image_tasks (
          id TEXT PRIMARY KEY,
          project_id TEXT NOT NULL,
          mode TEXT NOT NULL,
          source_script TEXT,
          visual_style TEXT,
          image_goal TEXT,
          stage TEXT NOT NULL DEFAULT 'idle',
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          FOREIGN KEY (project_id) REFERENCES projects(id)
        );

        CREATE TABLE IF NOT EXISTS image_outputs (
          id TEXT PRIMARY KEY,
          task_id TEXT NOT NULL,
          sections_json TEXT,
          raw_response TEXT,
          created_at TEXT NOT NULL,
          FOREIGN KEY (task_id) REFERENCES image_tasks(id)
        );

        CREATE TABLE IF NOT EXISTS video_tasks (
          id TEXT PRIMARY KEY,
          project_id TEXT NOT NULL,
          mode TEXT NOT NULL,
          script_beats TEXT,
          video_style TEXT,
          motion_focus TEXT,
          stage TEXT NOT NULL DEFAULT 'idle',
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          FOREIGN KEY (project_id) REFERENCES projects(id)
        );

        CREATE TABLE IF NOT EXISTS video_outputs (
          id TEXT PRIMARY KEY,
          task_id TEXT NOT NULL,
          sections_json TEXT,
          raw_response TEXT,
          created_at TEXT NOT NULL,
          FOREIGN KEY (task_id) REFERENCES video_tasks(id)
        );

        CREATE TABLE IF NOT EXISTS video_review_records (
          id TEXT PRIMARY KEY,
          task_id TEXT NOT NULL,
          score INTEGER,
          status TEXT NOT NULL,
          summary TEXT,
          issues_json TEXT,
          suggestions_json TEXT,
          review_model TEXT,
          created_at TEXT NOT NULL,
          FOREIGN KEY (task_id) REFERENCES video_tasks(id)
        );

        CREATE TABLE IF NOT EXISTS image_review_records (
          id TEXT PRIMARY KEY,
          task_id TEXT NOT NULL,
          score INTEGER,
          status TEXT NOT NULL,
          summary TEXT,
          issues_json TEXT,
          suggestions_json TEXT,
          review_model TEXT,
          created_at TEXT NOT NULL,
          FOREIGN KEY (task_id) REFERENCES image_tasks(id)
        );

        CREATE TABLE IF NOT EXISTS review_records (
          id TEXT PRIMARY KEY,
          task_id TEXT NOT NULL,
          score INTEGER,
          status TEXT NOT NULL,
          summary TEXT,
          issues_json TEXT,
          suggestions_json TEXT,
          dimensions_json TEXT,
          priority_json TEXT,
          rewrite_example TEXT,
          review_model TEXT,
          surgery_table_json TEXT,
          revision_path_json TEXT,
          created_at TEXT NOT NULL,
          FOREIGN KEY (task_id) REFERENCES script_tasks(id)
        );

        CREATE TABLE IF NOT EXISTS app_settings (
          id TEXT PRIMARY KEY,
          setting_key TEXT NOT NULL UNIQUE,
          setting_value TEXT,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS asset_records (
          id TEXT PRIMARY KEY,
          task_id TEXT NOT NULL,
          asset_type TEXT NOT NULL,
          asset_data_json TEXT NOT NULL,
          created_at TEXT NOT NULL,
          FOREIGN KEY (task_id) REFERENCES script_tasks(id)
        );

        CREATE TABLE IF NOT EXISTS prompt_output_records (
          id TEXT PRIMARY KEY,
          task_id TEXT NOT NULL,
          grid_groups_json TEXT NOT NULL,
          seedance_groups_json TEXT NOT NULL,
          generation_model TEXT,
          created_at TEXT NOT NULL,
          FOREIGN KEY (task_id) REFERENCES script_tasks(id)
        );

        CREATE TABLE IF NOT EXISTS seedance_analysis (
          task_id TEXT PRIMARY KEY,
          paragraph_index_json TEXT NOT NULL,
          structure_type TEXT,
          emotion_map_json TEXT NOT NULL,
          units_plan_json TEXT NOT NULL,
          total_sec INTEGER,
          total_units INTEGER,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          FOREIGN KEY (task_id) REFERENCES script_tasks(id)
        );

        CREATE TABLE IF NOT EXISTS seedance_units (
          id TEXT PRIMARY KEY,
          task_id TEXT NOT NULL,
          unit_index INTEGER NOT NULL,
          duration_sec INTEGER,
          scene_type TEXT,
          sub_shot_count INTEGER,
          copy_area TEXT,
          note_area_json TEXT,
          status TEXT NOT NULL DEFAULT 'pending',
          retry_count INTEGER NOT NULL DEFAULT 0,
          error_message TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          UNIQUE(task_id, unit_index),
          FOREIGN KEY (task_id) REFERENCES script_tasks(id)
        );

        CREATE TABLE IF NOT EXISTS users (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          username TEXT NOT NULL UNIQUE,
          email TEXT NOT NULL DEFAULT '',
          password_hash TEXT NOT NULL,
          salt TEXT NOT NULL,
          token TEXT,
          refresh_token TEXT,
          created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        ",
    )?;

    Ok(())
}
