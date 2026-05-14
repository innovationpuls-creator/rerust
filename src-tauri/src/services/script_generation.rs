use rusqlite::Connection;
use serde_json::json;

use crate::db::crud::{self, ScriptGenerationInput, ScriptGenerationResult, ScriptSection};
use crate::llm::config::RuntimeConfig;
use crate::llm::server_proxy::{self, ServerLlmParams};

/// Build the generation payload via two-round LLM call (planning → writing).
async fn build_generation_payload(
    runtime_config: &RuntimeConfig,
    input: &ScriptGenerationInput,
    on_chunk: Option<&(dyn Fn(&str) + Send + Sync)>,
) -> Result<GenerationPayload, String> {
    let input_json = json!({
        "mode": input.mode,
        "duration": input.duration,
        "inputSummary": input.input_summary,
        "stylePreset": input.style_preset,
        "genres": input.genres,
        "audience": input.audience,
        "tone": input.tone,
        "ending": input.ending,
        "outputMode": input.output_mode,
        "episodes": input.episodes,
        "customStyle": input.custom_style,
    })
    .to_string();

    // Round 1: Planning (silent)
    let plan_result = server_proxy::request_server_llm(ServerLlmParams {
        runtime_config: RuntimeConfig { ..runtime_config.clone() },
        prompt_slug: "script_planning".into(),
        user_messages: vec![json!({"role": "user", "content": input_json})],
        temperature: Some(0.7),
    })
    .await?;

    // Round 2: Writing
    let writing_messages = vec![
        json!({"role": "user", "content": plan_result}),
        json!({"role": "user", "content": input_json}),
    ];

    let script_completion = if let Some(cb) = on_chunk {
        server_proxy::request_server_llm_stream(
            ServerLlmParams {
                runtime_config: RuntimeConfig { ..runtime_config.clone() },
                prompt_slug: "script_writing".into(),
                user_messages: writing_messages,
                temperature: Some(0.7),
            },
            |chunk| cb(chunk),
        )
        .await?
    } else {
        server_proxy::request_server_llm(ServerLlmParams {
            runtime_config: RuntimeConfig { ..runtime_config.clone() },
            prompt_slug: "script_writing".into(),
            user_messages: writing_messages,
            temperature: Some(0.7),
        })
        .await?
    };

    Ok(GenerationPayload {
        sections: vec![ScriptSection {
            title: "完整结果文本".into(),
            content: script_completion,
        }],
        characters: vec![],
        raw_response: None,
    })
}

struct GenerationPayload {
    sections: Vec<ScriptSection>,
    characters: Vec<serde_json::Value>,
    raw_response: Option<serde_json::Value>,
}

/// Main entry: run full script generation (planning + writing).
pub async fn run_script_generation(
    conn: &Connection,
    input: &ScriptGenerationInput,
    on_chunk: Option<&(dyn Fn(&str) + Send + Sync)>,
) -> Result<ScriptGenerationResult, String> {
    let settings = crud::get_app_settings(conn);
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

    let payload = build_generation_payload(&runtime_config, input, on_chunk).await?;
    let result = crud::save_script_generation(
        conn,
        input,
        payload.sections,
        payload.characters,
        payload.raw_response,
    );
    Ok(result)
}

/// Run script generation with a pre-resolved runtime config (for async calls).
pub async fn run_script_generation_with_config(
    conn: &Connection,
    input: &ScriptGenerationInput,
    runtime_config: &RuntimeConfig,
    on_chunk: Option<&(dyn Fn(&str) + Send + Sync)>,
) -> Result<ScriptGenerationResult, String> {
    let payload = build_generation_payload(runtime_config, input, on_chunk).await?;
    let result = crud::save_script_generation(
        conn,
        input,
        payload.sections,
        payload.characters,
        payload.raw_response,
    );
    Ok(result)
}
