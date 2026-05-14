use std::collections::HashMap;

/// A parsed markdown block: `## TYPE [ID]` followed by `key: value` lines.
#[derive(Debug, Clone)]
pub struct Block {
    pub type_str: String,
    pub id: String,
    pub fields: HashMap<String, String>,
    pub raw: String,
    pub body: String,
}

/// Parse markdown into typed blocks.
pub fn parse_typed_blocks(markdown: &str) -> Vec<Block> {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut segments: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in &lines {
        if line.starts_with("## ") || line.starts_with("##\t") {
            if !current.is_empty() {
                segments.push(current);
            }
            current = vec![line];
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }

    segments
        .iter()
        .filter_map(|seg| {
            let text = seg.join("\n").trim().to_string();
            if text.is_empty() {
                return None;
            }
            parse_single_block(&text)
        })
        .collect()
}

fn parse_single_block(segment: &str) -> Option<Block> {
    let lines: Vec<&str> = segment.lines().collect();
    if lines.is_empty() {
        return None;
    }

    let header_line = lines[0].trim();
    let type_id = parse_header(header_line)?;
    let (type_str, id) = type_id;

    let mut body_lines: Vec<&str> = Vec::new();
    let mut kv_lines: Vec<&str> = Vec::new();
    let mut in_body = false;

    for line in &lines[1..] {
        let trimmed = line.trim();
        if !in_body && (trimmed.starts_with("---") || trimmed.starts_with("—")) {
            in_body = true;
            continue;
        }
        if in_body {
            body_lines.push(line);
        } else {
            kv_lines.push(line);
        }
    }

    let mut fields = HashMap::new();
    for line in kv_lines {
        if let Some((k, v)) = parse_kv_line(line) {
            fields.insert(k.to_string(), v.to_string());
        }
    }

    let body = body_lines.join("\n").trim().to_string();

    Some(Block {
        type_str: type_str.to_string(),
        id: id.to_string(),
        fields,
        raw: segment.to_string(),
        body,
    })
}

fn parse_header(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start_matches('#').trim();
    let parts: Vec<&str> = trimmed.splitn(2, char::is_whitespace).collect();
    let type_str = parts[0].to_string();
    let id = parts.get(1).unwrap_or(&"").trim().to_string();
    Some((type_str, id))
}

fn parse_kv_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("---")
        || trimmed.starts_with("===")
        || trimmed.starts_with("—")
    {
        return None;
    }
    let split = if let Some(pos) = trimmed.find(':') {
        pos
    } else if let Some(pos) = trimmed.find('：') {
        pos
    } else {
        return None;
    };
    let key = trimmed[..split].trim();
    let value = trimmed[split + 1..].trim();
    if key.is_empty() {
        return None;
    }
    Some((key.to_string(), value.to_string()))
}

// ── Step parsers ──

pub fn parse_step1(md: &str) -> serde_json::Value {
    let blocks = parse_typed_blocks(md);

    let meta = blocks.iter().find(|b| is_type_match(&b.type_str, "META"));
    let branch = get_field_from_map_opt(
        meta.map(|b| &b.fields).unwrap_or(&HashMap::new()),
        &["branch", "分支"],
    )
    .unwrap_or_default();

    let guidance = get_field_from_map_opt(
        meta.map(|b| &b.fields).unwrap_or(&HashMap::new()),
        &["guidance", "引导"],
    );

    let options: Vec<serde_json::Value> = blocks
        .iter()
        .filter(|b| is_type_match(&b.type_str, "PREMISE"))
        .enumerate()
        .map(|(i, b)| {
            serde_json::json!({
                "id": if b.id.is_empty() { format!("p{}", i + 1) } else { b.id.clone() },
                "title": get_field(b, &["title", "标题"]),
                "protagonist": get_field(b, &["protagonist", "主角"]),
                "want": get_field(b, &["want", "欲望", "需求"]),
                "obstacle": get_field(b, &["obstacle", "阻碍"]),
                "logline": get_field(b, &["logline", "一句话"]),
                "openingHook": get_field_opt(b, &["openingHook"]),
            })
        })
        .collect();

    serde_json::json!({
        "branch": branch,
        "options": options,
        "guidance": guidance,
    })
}

pub fn parse_step2(md: &str) -> serde_json::Value {
    let blocks = parse_typed_blocks(md);
    let synopsis = blocks.iter().find(|b| is_type_match(&b.type_str, "SYNOPSIS"));

    let text = synopsis
        .map(|b| {
            if !b.body.is_empty() {
                b.body.clone()
            } else {
                get_field(b, &["text"])
            }
        })
        .unwrap_or_default();

    let tone = synopsis
        .and_then(|b| get_field_opt(b, &["tone", "基调"]))
        .unwrap_or_default();

    serde_json::json!({
        "text": text,
        "charCount": text.chars().count(),
        "tone": tone,
    })
}

pub fn parse_step3(md: &str) -> serde_json::Value {
    let blocks = parse_typed_blocks(md);
    let characters: Vec<serde_json::Value> = blocks
        .iter()
        .filter(|b| is_type_match(&b.type_str, "CHARACTER"))
        .enumerate()
        .map(|(i, b)| {
            let role_raw = get_field(b, &["role", "角色"]);
            let role = match role_raw.as_str() {
                "主角" | "配角" | "反派" => role_raw,
                _ => "配角".to_string(),
            };

            let freq_words: Vec<String> = if !b.body.is_empty() {
                parse_list(&b.body)
            } else {
                parse_csv(&get_field(b, &["freqWords", "口头禅"]))
            };

            serde_json::json!({
                "id": if b.id.is_empty() { format!("c{}", i + 1) } else { b.id.clone() },
                "name": get_field(b, &["name", "姓名"]),
                "role": role,
                "want": get_field(b, &["want", "想要"]),
                "need": get_field(b, &["need", "真正需要"]),
                "arc": get_field(b, &["arc", "弧光"]),
                "bioKey": get_field(b, &["bioKey", "背景关键"]),
                "contradiction": get_field(b, &["contradiction", "内在矛盾"]),
                "linguistics": serde_json::json!({
                    "freqWords": freq_words,
                    "catchphrase": get_field(b, &["catchphrase", "标志性口头禅"]),
                    "gesture": get_field(b, &["gesture", "标志性动作"]),
                }),
            })
        })
        .collect();

    serde_json::json!({ "characters": characters })
}

pub fn parse_step4(md: &str) -> serde_json::Value {
    let blocks = parse_typed_blocks(md);
    let backstory = blocks.iter().find(|b| is_type_match(&b.type_str, "BACKSTORY"));

    let f = backstory.map(|b| &b.fields).cloned().unwrap_or_default();

    serde_json::json!({
        "era": get_field_from_map(&f, &["era", "时代"]),
        "protagonistGhost": get_field_from_map(&f, &["protagonistGhost", "主角前史"]),
        "relationPast": get_field_from_map(&f, &["relationPast", "关系既往"]),
        "crossSection": get_field_from_map(&f, &["crossSection", "横截面"]),
    })
}

pub fn parse_step5(md: &str) -> serde_json::Value {
    let blocks = parse_typed_blocks(md);

    let find_act = |act_num: u8| -> Option<&Block> {
        let cn = ["一", "二", "三", "四"];
        let act_str = format!("ACT{}", act_num);
        let cn_str = format!("第{}幕", cn[(act_num - 1) as usize]);
        blocks.iter().find(|b| {
            let t = b.type_str.to_uppercase();
            t == act_str || (t == "ACT" && b.id == act_num.to_string()) || b.type_str.contains(&cn_str)
        })
    };

    let a1 = find_act(1).map(|b| &b.fields).cloned().unwrap_or_default();
    let a2 = find_act(2).map(|b| &b.fields).cloned().unwrap_or_default();
    let a3 = find_act(3).map(|b| &b.fields).cloned().unwrap_or_default();
    let a4 = find_act(4).map(|b| &b.fields).cloned().unwrap_or_default();

    serde_json::json!({
        "act1": {
            "hook": get_field_from_map(&a1, &["hook", "开场钩子"]),
            "setup": get_field_from_map(&a1, &["setup", "建置"]),
            "incitingIncident": get_field_from_map(&a1, &["incitingIncident", "激励事件"]),
        },
        "act2": {
            "rise1": get_field_from_map(&a2, &["rise1", "冲突升级1"]),
            "rise2": get_field_from_map(&a2, &["rise2", "冲突升级2"]),
            "midpoint": get_field_from_map(&a2, &["midpoint", "中点"]),
        },
        "act3": {
            "climax": get_field_from_map(&a3, &["climax", "最终对决"]),
            "turnaround": get_field_from_map(&a3, &["turnaround", "关键反转"]),
        },
        "act4": {
            "newNormal": get_field_from_map(&a4, &["newNormal", "新常态"]),
        },
    })
}

pub fn parse_step6(md: &str) -> serde_json::Value {
    let blocks = parse_typed_blocks(md);
    let scenes: Vec<serde_json::Value> = blocks
        .iter()
        .filter(|b| is_scene_block(&b.type_str))
        .enumerate()
        .map(|(i, b)| {
            let id_num: u32 = b
                .id
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or((i + 1) as u32);

            serde_json::json!({
                "id": get_field_opt(b, &["id"]).unwrap_or_else(|| format!("s{}", id_num)),
                "index": id_num,
                "title": get_field(b, &["title", "标题"]),
                "locationTime": get_field(b, &["locationTime", "地点时间"]),
                "durationSec": parse_int_safe(&get_field(b, &["durationSec", "时长秒"]), 0),
                "plotRhythm": normalize_rhythm(&get_field(b, &["plotRhythm", "情节节奏"]), "中"),
                "emotionRhythm": normalize_emotion(&get_field(b, &["emotionRhythm", "情感节奏"]), "中"),
                "coreAction": get_field(b, &["coreAction", "核心动作"]),
            })
        })
        .collect();

    serde_json::json!({ "scenes": scenes })
}

pub fn parse_step7(md: &str) -> serde_json::Value {
    let blocks = parse_typed_blocks(md);
    let scenes: Vec<serde_json::Value> = blocks
        .iter()
        .filter(|b| is_scene_block(&b.type_str))
        .enumerate()
        .map(|(i, b)| {
            let id_num: u32 = b
                .id
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or((i + 1) as u32);

            serde_json::json!({
                "index": id_num,
                "header": get_field(b, &["header", "场景头"]),
                "duration": get_field(b, &["duration", "时长"]),
                "plotRhythm": normalize_rhythm(&get_field(b, &["plotRhythm", "情节节奏"]), "中"),
                "emotionRhythm": normalize_emotion(&get_field(b, &["emotionRhythm", "情感节奏"]), "中"),
                "body": b.body.clone(),
            })
        })
        .collect();

    serde_json::json!({ "scenes": scenes })
}

pub fn parse_step8(md: &str) -> serde_json::Value {
    let blocks = parse_typed_blocks(md);

    let doctor = blocks.iter().find(|b| is_type_match(&b.type_str, "DOCTOR"));
    let total_score = doctor
        .and_then(|b| parse_int_safe_opt(&get_field(b, &["totalScore", "总分"])))
        .unwrap_or(0);
    let verdict = get_field_opt(doctor.unwrap_or(&create_empty()), &["verdict", "结论"])
        .unwrap_or_default();

    let issues: Vec<String> = doctor.map(|b| parse_list(&b.body)).unwrap_or_default();

    let dimensions: Vec<serde_json::Value> = blocks
        .iter()
        .filter(|b| is_type_match(&b.type_str, "DIMENSION"))
        .map(|b| {
            serde_json::json!({
                "name": get_field(b, &["name", "维度名"]),
                "score": parse_int_safe(&get_field(b, &["score", "分数"]), 0),
                "comment": get_field_opt(b, &["comment", "评语"]),
            })
        })
        .collect();

    let surgery: Vec<serde_json::Value> = blocks
        .iter()
        .filter(|b| is_type_match(&b.type_str, "SURGERY"))
        .enumerate()
        .map(|(i, b)| {
            serde_json::json!({
                "id": if b.id.is_empty() { format!("sg{}", i + 1) } else { b.id.clone() },
                "original": get_field(b, &["original", "原文"]),
                "diagnosis": get_field(b, &["diagnosis", "诊断"]),
                "rewrite": get_field(b, &["rewrite", "重写"]),
            })
        })
        .collect();

    let rev_path = blocks.iter().find(|b| {
        is_type_match(&b.type_str, "REVISION_PATH")
            || is_type_match(&b.type_str, "REVISIONPATH")
    });
    let revision_path = rev_path.map(|b| parse_list(&b.body)).unwrap_or_default();

    serde_json::json!({
        "totalScore": total_score,
        "verdict": verdict,
        "dimensions": dimensions,
        "issues": issues,
        "surgery": surgery,
        "revisionPath": revision_path,
    })
}

pub fn parse_selfcheck(md: &str) -> Vec<serde_json::Value> {
    let blocks = parse_typed_blocks(md);
    blocks
        .iter()
        .filter(|b| is_type_match(&b.type_str, "CHECK") || is_type_match(&b.type_str, "检查点"))
        .enumerate()
        .map(|(i, b)| {
            let status_raw = get_field(b, &["status", "状态"]);
            let status = match status_raw.to_lowercase().as_str() {
                "pass" | "warn" | "fail" => status_raw.to_lowercase(),
                _ => "pass".to_string(),
            };

            let mut item = serde_json::json!({
                "id": if b.id.is_empty() { format!("{}", i + 1) } else { b.id.clone() },
                "label": get_field(b, &["label", "检查点"]),
                "status": status,
            });

            if let Some(issue) = get_field_opt(b, &["issue", "问题"]) {
                item["issue"] = serde_json::Value::String(issue);
            }
            if let Some(suggestion) = get_field_opt(b, &["suggestion", "建议"]) {
                item["suggestion"] = serde_json::Value::String(suggestion);
            }

            item
        })
        .collect()
}

/// Detect if step output is JSON or markdown, route to appropriate parser.
pub fn parse_step_output(step_number: u8, text: &str) -> Option<serde_json::Value> {
    let cleaned = strip_markdown_fences(text);
    // Some models (MiniMax, DeepSeek) wrap chain-of-thought in <think> blocks
    let stripped = strip_think_blocks(&cleaned);

    if looks_like_json(&stripped) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&stripped) {
            return Some(v);
        }
    }

    match step_number {
        1 => Some(parse_step1(&stripped)),
        2 => Some(parse_step2(&stripped)),
        3 => Some(parse_step3(&stripped)),
        4 => Some(parse_step4(&stripped)),
        5 => Some(parse_step5(&stripped)),
        6 => Some(parse_step6(&stripped)),
        7 => Some(parse_step7(&stripped)),
        8 => Some(parse_step8(&stripped)),
        _ => None,
    }
}

/// Remove `<think>...</think>` chain-of-thought blocks from model output.
fn strip_think_blocks(s: &str) -> String {
    let re = regex_lite::Regex::new(r"(?s)<think>.*?</think>").unwrap();
    re.replace_all(s, "").trim().to_string()
}

// ── Helpers ──

fn is_type_match(type_str: &str, target: &str) -> bool {
    type_str.to_uppercase() == target.to_uppercase()
}

fn is_scene_block(type_str: &str) -> bool {
    let upper = type_str.to_uppercase();
    upper == "SCENE" || type_str == "场景"
}

fn get_field(block: &Block, keys: &[&str]) -> String {
    get_field_opt(block, keys).unwrap_or_default()
}

fn get_field_opt(block: &Block, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(v) = block.fields.get(*key) {
            if !v.is_empty() {
                return Some(v.clone());
            }
        }
    }
    None
}

fn get_field_from_map(map: &HashMap<String, String>, keys: &[&str]) -> String {
    for key in keys {
        if let Some(v) = map.get(*key) {
            if !v.is_empty() {
                return v.clone();
            }
        }
    }
    String::new()
}

fn get_field_from_map_opt(map: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(v) = map.get(*key) {
            if !v.is_empty() {
                return Some(v.clone());
            }
        }
    }
    None
}

fn normalize_rhythm(val: &str, default: &str) -> String {
    match val {
        "松" | "中" | "紧" => val.to_string(),
        _ => default.to_string(),
    }
}

fn normalize_emotion(val: &str, default: &str) -> String {
    match val {
        "轻" | "中" | "重" => val.to_string(),
        _ => default.to_string(),
    }
}

fn parse_int_safe(s: &str, fallback: u32) -> u32 {
    s.parse().unwrap_or(fallback)
}

fn parse_int_safe_opt(s: &str) -> Option<u32> {
    s.parse().ok()
}

fn parse_list(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with('-')
                || trimmed.starts_with('*')
                || trimmed.starts_with('•')
                || trimmed.starts_with('·')
            {
                Some(trimmed[1..].trim().to_string())
            } else {
                None
            }
        })
        .collect()
}

fn parse_csv(s: &str) -> Vec<String> {
    s.split(|c: char| c == ',' || c == '，' || c == ';' || c == '；')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn strip_markdown_fences(s: &str) -> String {
    let re = regex_lite::Regex::new(r"(?m)^\s*```(?:json|JSON)?\s*$").unwrap();
    let result = re.replace_all(s, "");
    result.trim().to_string()
}

fn looks_like_json(s: &str) -> bool {
    let t = s.trim();
    t.starts_with('{') || t.starts_with('[')
}

fn create_empty() -> Block {
    Block {
        type_str: String::new(),
        id: String::new(),
        fields: HashMap::new(),
        raw: String::new(),
        body: String::new(),
    }
}
