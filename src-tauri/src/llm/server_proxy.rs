use crate::llm::config::RuntimeConfig;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ContextualLlmParams {
    pub runtime_config: RuntimeConfig,
    pub context_type: String,
    pub context_params: serde_json::Value,
    pub temperature: Option<f32>,
    pub max_tokens_override: Option<u32>,
}

#[derive(Debug)]
pub struct ServerLlmParams {
    pub runtime_config: RuntimeConfig,
    pub prompt_slug: String,
    pub user_messages: Vec<serde_json::Value>,
    pub temperature: Option<f32>,
}

/// Non-streaming server LLM call: builds prompt via build_prompt_slug(),
/// calls the LLM directly, returns the full response text.
pub async fn request_server_llm(params: ServerLlmParams) -> Result<String, String> {
    let mut full = String::new();
    {
        let mut collect = |chunk: &str| full.push_str(chunk);
        request_server_llm_inner(&params, &mut collect).await?;
    }
    Ok(full)
}

/// Streaming server LLM call: builds prompt via build_prompt_slug(),
/// calls the LLM directly, delivering chunks to on_chunk callback.
/// Returns the full response text.
pub async fn request_server_llm_stream(
    params: ServerLlmParams,
    on_chunk: impl FnMut(&str),
) -> Result<String, String> {
    request_server_llm_inner(&params, on_chunk).await
}

async fn request_server_llm_inner(
    params: &ServerLlmParams,
    mut on_chunk: impl FnMut(&str),
) -> Result<String, String> {
    let (system_prompt, user_messages) =
        crate::llm::prompts::build_prompt_slug(&params.prompt_slug, &params.user_messages);

    let mut messages = vec![serde_json::json!({"role": "system", "content": system_prompt})];
    messages.extend(user_messages);

    let cfg = &params.runtime_config;
    let model = &cfg.default_model;
    let mode = &cfg.text_mode;
    let temperature = resolve_temperature(model, params.temperature);

    if cfg.api_key.is_empty() || cfg.api_base_url.is_empty() {
        return Err("API 未配置，无法发起远程调用。".into());
    }

    match mode.as_str() {
        "gemini" => request_gemini(cfg, model, &messages, temperature, &mut on_chunk).await,
        "anthropic" => request_anthropic_stream(cfg, model, &messages, temperature, &mut on_chunk).await,
        _ => request_openai_stream(cfg, model, &messages, temperature, &mut on_chunk, 8192).await,
    }
}

pub async fn request_contextual_llm_stream(
    params: ContextualLlmParams,
    on_chunk: impl FnMut(&str),
) -> Result<String, String> {
    let (system_prompt, user_prompt) =
        crate::llm::prompts::build_prompt(&params.context_type, &params.context_params);

    let messages = vec![
        serde_json::json!({"role": "system", "content": system_prompt}),
        serde_json::json!({"role": "user", "content": user_prompt}),
    ];

    let cfg = &params.runtime_config;
    let model = &cfg.default_model;
    let mode = &cfg.text_mode;
    let temperature = resolve_temperature(model, params.temperature);
    let max_tokens = params.max_tokens_override.unwrap_or(8192);

    if cfg.api_key.is_empty() || cfg.api_base_url.is_empty() {
        return Err("API 未配置，无法发起远程调用。".into());
    }

    match mode.as_str() {
        "gemini" => request_gemini(cfg, model, &messages, temperature, on_chunk).await,
        "anthropic" => request_anthropic_stream(cfg, model, &messages, temperature, on_chunk).await,
        _ => request_openai_stream(cfg, model, &messages, temperature, on_chunk, max_tokens).await,
    }
}

/// Test connectivity to a configured LLM endpoint.
/// Returns (ok, latency_ms, error_message).
pub async fn test_connection(
    endpoint: &str,
    key: &str,
    model: &str,
    mode: &str,
) -> Result<(bool, u64, String), String> {
    let start = std::time::Instant::now();
    let url = build_text_url(endpoint, model, mode);
    let headers = build_headers(key, mode);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("LLM 客户端初始化失败：{}", e))?;

    let body = match mode {
        "gemini" => build_gemini_body(&[serde_json::json!({"role": "user", "content": "hi"})]),
        "anthropic" => serde_json::json!({
            "model": model,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "hi"}]
        }),
        _ => serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 1
        }),
    };

    let full_url = if mode == "gemini" {
        let sep = if url.contains('?') { "&" } else { "?" };
        format!("{}{}key={}", url, sep, urlencoding(key))
    } else {
        url.clone()
    };

    let response = client
        .post(&full_url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("网络错误：{}", e))?;

    let latency = start.elapsed().as_millis() as u64;
    let status = response.status();

    if status.is_success() {
        return Ok((true, latency, String::new()));
    }

    let detail = response.text().await.unwrap_or_default();
    let upstream = extract_upstream_error(&detail);

    let msg = match status.as_u16() {
        400 => {
            if looks_like_model_error(&detail) {
                format!("模型名无效：{}", upstream.unwrap_or_else(|| "上游拒绝该模型".into()))
            } else {
                format!("请求参数错误（400）{}", upstream_msg(upstream))
            }
        }
        401 => format!("API Key 无效或已过期{}", upstream_msg(upstream)),
        403 => format!("权限不足或配额耗尽{}", upstream_msg(upstream)),
        404 => format!("端点路径不存在（404）{}", upstream_msg(upstream)),
        429 => format!("请求过频或配额耗尽（429）{}", upstream_msg(upstream)),
        _ if status.as_u16() >= 500 => {
            format!("上游服务异常（{}）{}", status.as_u16(), upstream_msg(upstream))
        }
        _ => format!("HTTP {}{}", status.as_u16(), upstream_msg(upstream)),
    };

    Ok((false, latency, msg))
}

// ── URL Building ──

fn normalize_endpoint(endpoint: &str) -> String {
    endpoint.trim().trim_end_matches('/').to_string()
}

fn build_text_url(endpoint: &str, model: &str, mode: &str) -> String {
    let base = normalize_endpoint(endpoint);
    match mode {
        "gemini" => format!("{}/models/{}:generateContent", base, model),
        "anthropic" => {
            if base.ends_with("/messages") {
                base
            } else {
                format!("{}/messages", base)
            }
        }
        _ => {
            if base.ends_with("/chat/completions") {
                base
            } else {
                format!("{}/chat/completions", base)
            }
        }
    }
}

// ── Headers ──

fn build_headers(key: &str, mode: &str) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        "application/json".parse().unwrap(),
    );
    match mode {
        "anthropic" => {
            headers.insert(
                reqwest::header::HeaderName::from_static("x-api-key"),
                key.parse().unwrap(),
            );
            headers.insert(
                reqwest::header::HeaderName::from_static("anthropic-version"),
                "2023-06-01".parse().unwrap(),
            );
        }
        "gemini" => {
            // key goes in query param, not header
        }
        _ => {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", key).parse().unwrap(),
            );
        }
    }
    headers
}

// ── Request body builders ──

fn build_openai_body(
    model: &str,
    messages: &[serde_json::Value],
    temperature: f32,
    stream: bool,
    max_tokens: u32,
) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "messages": messages,
        "temperature": temperature,
        "stream": stream,
        "max_tokens": max_tokens,
    })
}

fn build_anthropic_body(
    model: &str,
    messages: &[serde_json::Value],
    temperature: f32,
    stream: bool,
) -> serde_json::Value {
    let system = messages
        .iter()
        .find(|m| m["role"] == "system")
        .and_then(|m| m["content"].as_str())
        .unwrap_or("");
    let user_messages: Vec<serde_json::Value> = messages
        .iter()
        .filter(|m| m["role"] != "system")
        .cloned()
        .collect();

    serde_json::json!({
        "model": model,
        "max_tokens": 8192,
        "temperature": temperature,
        "system": system,
        "messages": user_messages,
        "stream": stream,
    })
}

fn build_gemini_body(messages: &[serde_json::Value]) -> serde_json::Value {
    let contents: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            let text = m["content"].as_str().unwrap_or("");
            serde_json::json!({
                "role": "user",
                "parts": [{"text": text}]
            })
        })
        .collect();
    serde_json::json!({ "contents": contents })
}

// ── API callers ──

async fn request_openai_stream(
    cfg: &RuntimeConfig,
    model: &str,
    messages: &[serde_json::Value],
    temperature: f32,
    mut on_chunk: impl FnMut(&str),
    max_tokens: u32,
) -> Result<String, String> {
    let url = build_text_url(&cfg.api_base_url, model, "openai");
    let headers = build_headers(&cfg.api_key, "openai");
    let body = build_openai_body(model, messages, temperature, true, max_tokens);

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("OpenAI 调用失败：{}", e))?;

    let status = response.status();
    if !status.is_success() {
        let detail = response.text().await.unwrap_or_default();
        return Err(format!("OpenAI 调用失败（{}）：{}", status.as_u16(), &detail[..detail.len().min(200)]));
    }

    parse_openai_sse(response, &mut on_chunk).await
}

async fn request_anthropic_stream(
    cfg: &RuntimeConfig,
    model: &str,
    messages: &[serde_json::Value],
    temperature: f32,
    mut on_chunk: impl FnMut(&str),
) -> Result<String, String> {
    let url = build_text_url(&cfg.api_base_url, model, "anthropic");
    let headers = build_headers(&cfg.api_key, "anthropic");
    let body = build_anthropic_body(model, messages, temperature, true);

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Anthropic 调用失败：{}", e))?;

    let status = response.status();
    if !status.is_success() {
        let detail = response.text().await.unwrap_or_default();
        return Err(format!(
            "Anthropic 调用失败（{}）：{}",
            status.as_u16(),
            &detail[..detail.len().min(200)]
        ));
    }

    parse_anthropic_sse(response, &mut on_chunk).await
}

async fn request_gemini(
    cfg: &RuntimeConfig,
    model: &str,
    messages: &[serde_json::Value],
    _temperature: f32,
    on_chunk: impl FnMut(&str),
) -> Result<String, String> {
    let url = build_text_url(&cfg.api_base_url, model, "gemini");
    let headers = build_headers(&cfg.api_key, "gemini");
    let body = build_gemini_body(messages);

    let sep = if url.contains('?') { "&" } else { "?" };
    let full_url = format!("{}{}key={}", url, sep, urlencoding(&cfg.api_key));

    let client = reqwest::Client::new();
    let response = client
        .post(&full_url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Gemini 调用失败：{}", e))?;

    let status = response.status();
    if !status.is_success() {
        let detail = response.text().await.unwrap_or_default();
        return Err(format!(
            "Gemini 调用失败（{}）：{}",
            status.as_u16(),
            &detail[..detail.len().min(200)]
        ));
    }

    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Gemini 响应解析失败：{}", e))?;

    let text = payload["candidates"]
        .as_array()
        .and_then(|c| c.first())
        .and_then(|c| c["content"]["parts"].as_array())
        .and_then(|parts| {
            parts
                .iter()
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
                .into_option()
        })
        .unwrap_or_default();

    if text.is_empty() {
        return Err("Gemini 返回了空内容。".into());
    }

    // Gemini doesn't support streaming — deliver full result at once
    // We need interior mutability for the callback since we're not streaming
    let mut cb = on_chunk;
    cb(&text);
    Ok(text)
}

// ── SSE Parsing ──

async fn parse_openai_sse(
    response: reqwest::Response,
    on_chunk: &mut impl FnMut(&str),
) -> Result<String, String> {
    use futures::StreamExt;

    let stream = response.bytes_stream();
    let mut full = String::new();
    let mut buffer = String::new();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut s = stream;
        while let Some(chunk) = s.next().await {
            match chunk {
                Ok(bytes) => {
                    let _ = tx.send(bytes);
                }
                Err(e) => {
                    log::error!("SSE stream error: {}", e);
                    break;
                }
            }
        }
    });

    use tokio_stream::wrappers::UnboundedReceiverStream;
    let mut rx = UnboundedReceiverStream::new(rx);

    while let Some(bytes) = rx.next().await {
        let text = String::from_utf8_lossy(&bytes);
        buffer.push_str(&text);

        let lines: Vec<String> = buffer.split('\n').map(String::from).collect();
        buffer = lines.last().cloned().unwrap_or_default();

        for line in lines.iter().take(lines.len().saturating_sub(1)) {
            let trimmed = line.trim();
            if !trimmed.starts_with("data:") {
                continue;
            }
            let data = trimmed[5..].trim();
            if data == "[DONE]" {
                continue;
            }
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(text) = parsed["choices"]
                    .as_array()
                    .and_then(|c| c.first())
                    .and_then(|c| c["delta"]["content"].as_str())
                {
                    full.push_str(text);
                    on_chunk(text);
                }
            }
        }
    }

    Ok(full.trim().to_string())
}

async fn parse_anthropic_sse(
    response: reqwest::Response,
    on_chunk: &mut impl FnMut(&str),
) -> Result<String, String> {
    use futures::StreamExt;

    let stream = response.bytes_stream();
    let mut full = String::new();
    let mut buffer = String::new();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut s = stream;
        while let Some(chunk) = s.next().await {
            match chunk {
                Ok(bytes) => {
                    let _ = tx.send(bytes);
                }
                Err(e) => {
                    log::error!("SSE stream error: {}", e);
                    break;
                }
            }
        }
    });

    use tokio_stream::wrappers::UnboundedReceiverStream;
    let mut rx = UnboundedReceiverStream::new(rx);

    while let Some(bytes) = rx.next().await {
        let text = String::from_utf8_lossy(&bytes);
        buffer.push_str(&text);

        let lines: Vec<String> = buffer.split('\n').map(String::from).collect();
        buffer = lines.last().cloned().unwrap_or_default();

        for line in lines.iter().take(lines.len().saturating_sub(1)) {
            let trimmed = line.trim();
            if !trimmed.starts_with("data:") {
                continue;
            }
            let data = trimmed[5..].trim();
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                if parsed["type"] == "content_block_delta" {
                    if let Some(text) = parsed["delta"]["text"].as_str() {
                        full.push_str(text);
                        on_chunk(text);
                    }
                }
            }
        }
    }

    Ok(full.trim().to_string())
}

// ── Utilities ──

fn resolve_temperature(model: &str, requested: Option<f32>) -> f32 {
    let name = model.to_lowercase();
    if name.starts_with("kimi-k2") {
        return 1.0;
    }
    requested.unwrap_or(0.8)
}

fn urlencoding(s: &str) -> String {
    // Simple URL encoding for API keys (only encode special chars)
    let mut result = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => result.push_str("%20"),
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

fn extract_upstream_error(detail: &str) -> Option<String> {
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(detail) {
        let e = parsed.get("error")?;
        let msg = match e {
            serde_json::Value::String(s) => Some(s.clone()),
            _ => e.get("message")
                .or_else(|| e.get("msg"))
                .or_else(|| e.get("reason"))
                .and_then(|v| v.as_str())
                .map(String::from),
        };
        return msg;
    }
    // Try top-level "message" field if no "error" field
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(detail) {
        return parsed.get("message").and_then(|v| v.as_str()).map(String::from);
    }
    None
}

fn upstream_msg(msg: Option<String>) -> String {
    match msg {
        Some(s) if !s.is_empty() => format!("：{}", &s[..s.len().min(200)]),
        _ => String::new(),
    }
}

fn looks_like_model_error(detail: &str) -> bool {
    let s = detail.to_lowercase();
    s.contains("model not found")
        || s.contains("model_not_found")
        || s.contains("no such model")
        || s.contains("invalid model")
        || s.contains("unknown model")
        || s.contains("model does not exist")
}

// Helper trait to convert Option<String> to Option<&str> for collect
trait IntoOption {
    fn into_option(self) -> Option<String>;
}

impl IntoOption for String {
    fn into_option(self) -> Option<String> {
        if self.is_empty() { None } else { Some(self) }
    }
}
