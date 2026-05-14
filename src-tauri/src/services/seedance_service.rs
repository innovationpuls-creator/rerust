use rusqlite::{params, Connection};
use serde_json::json;

use crate::llm::config::RuntimeConfig;
use crate::llm::server_proxy::{self, ContextualLlmParams};
use crate::utils::v5_parser::{self, NoteArea, V5Analysis, V5UnitPlan};

const TARGET_PARA_LEN: usize = 280;
const MAX_PARA_LEN: usize = 600;
const SECONDS_PER_UNIT: f64 = 13.5;

struct SceneHeader {
    scene_num: usize,
    secs: usize,
}

fn chinese_digit_to_number(s: &str) -> Option<usize> {
    if let Ok(n) = s.parse::<usize>() {
        return Some(n);
    }
    let map: std::collections::HashMap<char, usize> = [
        ('零', 0), ('一', 1), ('二', 2), ('三', 3), ('四', 4),
        ('五', 5), ('六', 6), ('七', 7), ('八', 8), ('九', 9), ('十', 10),
    ]
    .iter()
    .copied()
    .collect();
    let chars: Vec<char> = s.chars().collect();
    if chars.len() == 1 {
        return map.get(&chars[0]).copied();
    }
    if chars.len() == 2 && chars[0] == '十' {
        return Some(10 + map.get(&chars[1]).unwrap_or(&0));
    }
    if chars.len() == 2 && chars[1] == '十' {
        return Some(map.get(&chars[0]).unwrap_or(&0) * 10);
    }
    if chars.len() == 3 && chars[1] == '十' {
        return Some(map.get(&chars[0]).unwrap_or(&0) * 10 + map.get(&chars[2]).unwrap_or(&0));
    }
    None
}

fn parse_scene_secs(text: &str) -> Option<usize> {
    let re = regex_lite::Regex::new(r"[（(]\s*(\d+):(\d+)\s*[-–—~～]\s*(\d+):(\d+)\s*[)）]").unwrap();
    if let Some(cap) = re.captures(text) {
        let start = cap[1].parse::<usize>().unwrap_or(0) * 60 + cap[2].parse::<usize>().unwrap_or(0);
        let end = cap[3].parse::<usize>().unwrap_or(0) * 60 + cap[4].parse::<usize>().unwrap_or(0);
        if end > start {
            return Some(end - start);
        }
    }
    let re = regex_lite::Regex::new(r"[（(]\s*约?\s*(\d+)\s*分\s*(\d+)\s*秒\s*[)）]").unwrap();
    if let Some(cap) = re.captures(text) {
        return Some(cap[1].parse::<usize>().unwrap_or(0) * 60 + cap[2].parse::<usize>().unwrap_or(0));
    }
    let re = regex_lite::Regex::new(r"[（(]\s*约?\s*(\d+)\s*分钟?\s*[)）]").unwrap();
    if let Some(cap) = re.captures(text) {
        return Some(cap[1].parse::<usize>().unwrap_or(0) * 60);
    }
    let re = regex_lite::Regex::new(r"[（(]\s*约?\s*(\d+)\s*秒\s*[)）]").unwrap();
    if let Some(cap) = re.captures(text) {
        return Some(cap[1].parse::<usize>().unwrap_or(0));
    }
    None
}

fn try_parse_scene_header(line: &str) -> Option<SceneHeader> {
    let re_bracket =
        regex_lite::Regex::new(r"^【\s*(场景|场)\s*([一二三四五六七八九十百零\d]+)\s*[:：]\s*(.+?)\s*】").unwrap();
    if let Some(cap) = re_bracket.captures(line) {
        let num_str = cap.get(2).unwrap().as_str();
        let _title = cap.get(3).unwrap().as_str().trim().to_string();
        if let Some(secs) = parse_scene_secs(line) {
            if let Some(scene_num) = chinese_digit_to_number(num_str) {
                if scene_num > 0 {
                    return Some(SceneHeader { scene_num, secs });
                }
            }
        }
    }
    let re_md = regex_lite::Regex::new(
        r"^#{1,4}\s+(?:(?:场|场景)\s*([一二三四五六七八九十百零\d]+)|第\s*([一二三四五六七八九十百零\d]+)\s*场)\s*[:：]?\s*(.+?)$",
    )
    .unwrap();
    if let Some(cap) = re_md.captures(line) {
        let num_str = cap.get(1).or_else(|| cap.get(2)).map(|m| m.as_str()).unwrap_or("");
        let _title = cap.get(3).map(|m| m.as_str().trim()).unwrap_or("").to_string();
        if let Some(secs) = parse_scene_secs(line) {
            if let Some(scene_num) = chinese_digit_to_number(num_str) {
                if scene_num > 0 {
                    return Some(SceneHeader { scene_num, secs });
                }
            }
        }
    }
    let re_plain = regex_lite::Regex::new(
        r"^(?:(?:场|场景)\s*([一二三四五六七八九十百零\d]+)|第\s*([一二三四五六七八九十百零\d]+)\s*场)\s*[:：]\s*(.+?)$",
    )
    .unwrap();
    if let Some(cap) = re_plain.captures(line) {
        let num_str = cap.get(1).or_else(|| cap.get(2)).map(|m| m.as_str()).unwrap_or("");
        let _title = cap.get(3).map(|m| m.as_str().trim()).unwrap_or("").to_string();
        if let Some(secs) = parse_scene_secs(line) {
            if let Some(scene_num) = chinese_digit_to_number(num_str) {
                if scene_num > 0 {
                    return Some(SceneHeader { scene_num, secs });
                }
            }
        }
    }
    None
}

fn parse_init_duration_to_sec(duration: &str) -> usize {
    let re = regex_lite::Regex::new(r"(\d+)\s*分\s*(\d+)\s*秒").unwrap();
    if let Some(cap) = re.captures(duration) {
        return cap[1].parse::<usize>().unwrap_or(0) * 60 + cap[2].parse::<usize>().unwrap_or(0);
    }
    let re = regex_lite::Regex::new(r"(\d+)\s*分").unwrap();
    if let Some(cap) = re.captures(duration) {
        return cap[1].parse::<usize>().unwrap_or(0) * 60;
    }
    let re = regex_lite::Regex::new(r"(\d+)\s*秒").unwrap();
    if let Some(cap) = re.captures(duration) {
        return cap[1].parse::<usize>().unwrap_or(0);
    }
    60
}

struct AggregatedPara {
    text: String,
    scene_id: usize,
}

struct SceneMeta {
    scene_id: usize,
    unit_count: usize,
    section_refs: Vec<String>,
}

fn split_script_into_paragraphs(
    script_body: &str,
) -> (Vec<v5_parser::ParagraphIndexItem>, Vec<SceneMeta>, usize) {
    let raw_initial: Vec<String> = script_body
        .split("\n\n")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let mut raw_paras: Vec<String> = Vec::new();
    for para in &raw_initial {
        let first_line = para.lines().next().unwrap_or("");
        if try_parse_scene_header(first_line).is_some() {
            let header_end = para.find('\n').unwrap_or(para.len());
            let header_block = para[..header_end].trim().to_string();
            raw_paras.push(header_block);
            let remainder = if header_end < para.len() {
                para[header_end + 1..].trim().to_string()
            } else {
                String::new()
            };
            if !remainder.is_empty() {
                let mut body_lines: Vec<String> = Vec::new();
                for line in remainder.lines() {
                    let t = line.trim().to_string();
                    if t.is_empty() {
                        if !body_lines.is_empty() {
                            raw_paras.push(body_lines.join("\n"));
                            body_lines.clear();
                        }
                        continue;
                    }
                    let is_meta = regex_lite::Regex::new(r"^情节节奏[:：]|^情感节奏[:：]")
                        .unwrap()
                        .is_match(&t);
                    if is_meta {
                        if !body_lines.is_empty() {
                            raw_paras.push(body_lines.join("\n"));
                            body_lines.clear();
                        }
                        raw_paras.push(t);
                        continue;
                    }
                    body_lines.push(t);
                }
                if !body_lines.is_empty() {
                    raw_paras.push(body_lines.join("\n"));
                }
            }
        } else {
            raw_paras.push(para.clone());
        }
    }

    let mut scenes: Vec<SceneMeta> = Vec::new();
    let mut para_scene_ids: Vec<usize> = Vec::new();
    let mut current_scene_id: usize = 0;

    for para in &raw_paras {
        let first_line = para.lines().next().unwrap_or("");
        if let Some(header) = try_parse_scene_header(first_line) {
            let unit_count = std::cmp::max(1, (header.secs as f64 / SECONDS_PER_UNIT).round() as usize);
            current_scene_id = header.scene_num;
            scenes.push(SceneMeta {
                scene_id: current_scene_id,
                unit_count,
                section_refs: Vec::new(),
            });
            para_scene_ids.push(0);
        } else {
            let is_metadata = regex_lite::Regex::new(r"^情节节奏[:：]|^情感节奏[:：]")
                .unwrap()
                .is_match(para);
            if is_metadata {
                para_scene_ids.push(0);
            } else {
                para_scene_ids.push(current_scene_id);
            }
        }
    }

    let mut aggregated: Vec<AggregatedPara> = Vec::new();
    let mut buffer = String::new();
    let mut buffer_scene_id: usize = 0;

    for (i, para) in raw_paras.iter().enumerate() {
        let sid = para_scene_ids[i];
        if sid == 0 {
            continue;
        }
        if !buffer.is_empty() && sid != buffer_scene_id {
            aggregated.push(AggregatedPara {
                text: buffer.clone(),
                scene_id: buffer_scene_id,
            });
            buffer.clear();
            buffer_scene_id = 0;
        }
        if buffer.is_empty() {
            buffer = para.clone();
            buffer_scene_id = sid;
        } else {
            let merged = format!("{}\n\n{}", buffer, para);
            if merged.len() > MAX_PARA_LEN || buffer.len() >= TARGET_PARA_LEN {
                aggregated.push(AggregatedPara {
                    text: buffer.clone(),
                    scene_id: buffer_scene_id,
                });
                buffer = para.clone();
                buffer_scene_id = sid;
            } else {
                buffer = merged;
            }
        }
    }
    if !buffer.is_empty() {
        aggregated.push(AggregatedPara {
            text: buffer,
            scene_id: buffer_scene_id,
        });
    }

    let mut paragraphs: Vec<v5_parser::ParagraphIndexItem> = Vec::new();
    let mut scene_refs_map: std::collections::HashMap<usize, Vec<String>> =
        std::collections::HashMap::new();

    for (idx, agg) in aggregated.iter().enumerate() {
        let p_idx = idx + 1;
        let mut push_para =|id: String, text: String, sid: usize, refs_map: &mut std::collections::HashMap<usize, Vec<String>>| {
            paragraphs.push(v5_parser::ParagraphIndexItem {
                id: id.clone(),
                text,
                facts: None,
            });
            if sid > 0 {
                refs_map.entry(sid).or_default().push(id);
            }
        };

        if agg.text.len() <= MAX_PARA_LEN * 3 / 2 {
            push_para(format!("§{}", p_idx), agg.text.clone(), agg.scene_id, &mut scene_refs_map);
        } else {
            let re_sent = regex_lite::Regex::new(r"(?<=[。！？.?!])").unwrap();
            let sentences: Vec<&str> = re_sent
                .split(&agg.text)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();

            if sentences.len() <= 1 {
                push_para(format!("§{}", p_idx), agg.text.clone(), agg.scene_id, &mut scene_refs_map);
            } else {
                let mut sub_buf = String::new();
                let mut sub_idx = 0;
                for sent in sentences {
                    if !sub_buf.is_empty() && sub_buf.len() + sent.len() > MAX_PARA_LEN {
                        sub_idx += 1;
                        push_para(format!("§{}.{}", p_idx, sub_idx), sub_buf.clone(), agg.scene_id, &mut scene_refs_map);
                        sub_buf = sent.to_string();
                    } else {
                        sub_buf.push_str(sent);
                    }
                }
                if !sub_buf.is_empty() {
                    sub_idx += 1;
                    push_para(format!("§{}.{}", p_idx, sub_idx), sub_buf, agg.scene_id, &mut scene_refs_map);
                }
            }
        }
    }

    for scene in &mut scenes {
        if let Some(refs) = scene_refs_map.remove(&scene.scene_id) {
            scene.section_refs = refs;
        }
    }

    scenes.retain(|s| !s.section_refs.is_empty());
    let total_units: usize = scenes.iter().map(|s| s.unit_count).sum();

    (paragraphs, scenes, total_units)
}

fn load_script_body(conn: &Connection, task_id: &str) -> Result<String, String> {
    let row: Result<Option<String>, _> = conn.query_row(
        "SELECT script_body FROM script_outputs WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 1",
        params![task_id],
        |row| row.get(0),
    );
    match row {
        Ok(Some(body)) if !body.trim().is_empty() => Ok(body),
        _ => Err("当前剧本任务还没有正文内容。请先完成剧本阶段 (Step 7 写作)。".into()),
    }
}

fn load_duration(conn: &Connection, task_id: &str) -> String {
    conn.query_row(
        "SELECT duration FROM script_tasks WHERE id = ?1",
        params![task_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
    .unwrap_or_else(|| "8分钟".to_string())
}

fn load_assets_json(conn: &Connection, task_id: &str) -> String {
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
        if let Ok(mut data) = serde_json::from_str::<serde_json::Value>(data_str) {
            if let Some(obj) = data.as_object_mut() {
                obj.remove("id");
                obj.remove("aiPrompt");
            }
            match ty {
                "character" => {
                    data["ref"] = json!(format!("@C{}", characters.len() + 1));
                    characters.push(data);
                }
                "scene" => {
                    data["ref"] = json!(format!("@S{}", scenes.len() + 1));
                    scenes.push(data);
                }
                "prop" => {
                    data["ref"] = json!(format!("@P{}", props.len() + 1));
                    props.push(data);
                }
                _ => {}
            }
        }
    }
    json!({ "characters": characters, "scenes": scenes, "props": props }).to_string()
}

fn strip_fences(s: &str) -> String {
    let re_start = regex_lite::Regex::new(r"^\s*```(?:json|JSON)?\s*").unwrap();
    let re_end = regex_lite::Regex::new(r"```\s*$").unwrap();
    let s = re_start.replace(s, "");
    let s = re_end.replace(&s, "");
    s.trim().to_string()
}

fn repair_json(s: &str) -> String {
    let mut s = s.to_string();
    s = s.replace('\u{feff}', "").replace('\u{a0}', " ");
    s = s.replace('\u{201c}', "\"").replace('\u{201d}', "\"");
    s = s.replace('\u{2018}', "'").replace('\u{2019}', "'");
    s = s.replace(['\u{ff3b}', '\u{3010}'], "[").replace(['\u{ff3d}', '\u{3011}'], "]");
    s = s.replace('\u{ff5b}', "{").replace('\u{ff5d}', "}");

    let mut out = String::with_capacity(s.len());
    let mut in_str = false;
    let mut esc = false;
    let chars: Vec<char> = s.chars().collect();
    for i in 0..chars.len() {
        let c = chars[i];
        if esc {
            out.push(c);
            esc = false;
            continue;
        }
        if in_str {
            match c {
                '\\' => { out.push(c); esc = true; }
                '"' => {
                    let mut j = i + 1;
                    while j < chars.len() && chars[j].is_whitespace() { j += 1; }
                    let nx = if j < chars.len() { chars[j] } else { '\0' };
                    if nx == '\0' || nx == ',' || nx == ':' || nx == '}' || nx == ']' {
                        out.push('"');
                        in_str = false;
                    } else {
                        out.push('\\');
                        out.push('"');
                    }
                }
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                _ => out.push(c),
            }
        } else {
            match c {
                '\u{ff0c}' | '\u{3001}' => out.push(','),
                '\u{ff1a}' => out.push(':'),
                '\u{ff1b}' => out.push(';'),
                '"' => { in_str = true; out.push(c); }
                _ => out.push(c),
            }
        }
    }

    let re_trailing = regex_lite::Regex::new(r",(\s*[}\]])").unwrap();
    re_trailing.replace_all(&out, "$1").to_string()
}

#[allow(dead_code)]
fn try_parse_json(text: &str) -> Option<serde_json::Value> {
    let cleaned = strip_fences(text);
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&cleaned) {
        return Some(v);
    }
    let repaired = repair_json(&cleaned);
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&repaired) {
        return Some(v);
    }
    if let Some(start) = cleaned.find('{') {
        if let Some(end) = cleaned.rfind('}') {
            if end > start {
                let slice = &cleaned[start..=end];
                if let Ok(v) = serde_json::from_str(slice) {
                    return Some(v);
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&repair_json(slice)) {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn extract_script_fragment(
    paragraphs: &[v5_parser::ParagraphIndexItem],
    unit: &V5UnitPlan,
) -> String {
    let lookup: std::collections::HashMap<&str, &str> = paragraphs
        .iter()
        .map(|p| (p.id.as_str(), p.text.as_str()))
        .collect();
    unit.section_refs
        .iter()
        .filter_map(|ref_id| lookup.get(ref_id.as_str()).copied())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

// ═══════════════════════════════════════════════════════════════
// Phase A-D · Analysis
// ═══════════════════════════════════════════════════════════════

pub async fn run_phase_ad(
    conn: &Connection,
    task_id: &str,
) -> Result<V5Analysis, String> {
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
    let duration = load_duration(conn, task_id);
    let assets_json = load_assets_json(conn, task_id);

    let mut working_script = script_body.clone();
    let (mut paragraphs, mut scenes, mut total_units) = split_script_into_paragraphs(&working_script);

    if scenes.is_empty() {
        let total_sec = parse_init_duration_to_sec(&duration);
        working_script = format!("【场景1：全片】（约{}秒）\n\n{}", total_sec, script_body);
        let (p, s, tu) = split_script_into_paragraphs(&working_script);
        paragraphs = p;
        scenes = s;
        total_units = tu;
    }

    let has_scenes = !scenes.is_empty();

    let context_params = json!({
        "scriptBody": working_script,
        "duration": duration,
        "assetsJson": assets_json,
    });

    let full_text = server_proxy::request_contextual_llm_stream(
        ContextualLlmParams {
            runtime_config: RuntimeConfig { ..runtime_config.clone() },
            context_type: "seedance_phase_ad".into(),
            context_params,
            temperature: Some(0.3),
            max_tokens_override: None,
        },
        |_chunk| {},
    )
    .await?;

    let parsed = v5_parser::parse_v5_analysis(&full_text);

    if parsed.units.is_empty() {
        let diag = format!(
            "[切段诊断] script_body={}字 → 切出 {} 段, {} 场, totalUnits={}\n{}",
            script_body.len(),
            paragraphs.len(),
            scenes.len(),
            total_units,
            if paragraphs.is_empty() {
                format!("⚠ 切段为空! 检查剧本格式是否含场景头标注\n剧本前200字:\n{}", &script_body[..script_body.len().min(200)])
            } else {
                "切段正常但 LLM 返回 0 UNIT blocks".to_string()
            }
        );
        return Err(format!("Phase A-D 分析失败:\n{}", diag));
    }

    if has_scenes {
        let expected = total_units;
        let actual = parsed.units.len();
        if (actual as isize - expected as isize).abs() > 1 {
            log::warn!(
                "[seedance Phase D] LLM scene unit分配不符: 期望{} unit, 实际{}",
                expected,
                actual
            );
        }
    }

    crate::services::seedance_store::save_analysis(conn, task_id, &parsed);
    crate::services::seedance_store::delete_units(conn, task_id);

    Ok(parsed)
}

pub fn get_analysis(conn: &Connection, task_id: &str) -> Option<V5Analysis> {
    crate::services::seedance_store::load_analysis(conn, task_id)
}

// ═══════════════════════════════════════════════════════════════
// Phase E-F-G · Unit generation
// ═══════════════════════════════════════════════════════════════

pub async fn run_unit_generation(
    conn: &Connection,
    task_id: &str,
    unit_index: usize,
) -> Result<serde_json::Value, String> {
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

    let analysis = crate::services::seedance_store::load_analysis(conn, task_id)
        .ok_or_else(|| "请先生成 Phase A-D 分析 (点击生成分镜).".to_string())?;

    let unit = analysis
        .units
        .iter()
        .find(|u| u.index == unit_index)
        .cloned()
        .ok_or_else(|| format!("单元 #{} 不存在 (分析里只有 {} 个单元).", unit_index, analysis.units.len()))?;

    let plan_entry = unit.planned_entry_state.clone();
    let previous_unit_plan = if unit_index > 1 {
        analysis.units.iter().find(|u| u.index == unit_index - 1)
    } else {
        None
    };
    let previous_plan_exit = previous_unit_plan
        .map(|u| u.planned_exit_state.clone())
        .unwrap_or_default();

    let existing_record = crate::services::seedance_store::get_unit(conn, task_id, unit_index as i32);
    let retry_count = existing_record
        .and_then(|r| r["retryCount"].as_i64())
        .unwrap_or(0) as i32;

    crate::services::seedance_store::upsert_unit(
        conn,
        task_id,
        unit_index as i32,
        Some(unit.duration_sec as i32),
        &unit.scene_type,
        Some(unit.sub_shot_count as i32),
        "",
        &NoteArea::default(),
        "generating",
        retry_count,
        None,
    );

    let (paragraphs, _, _) = split_script_into_paragraphs(
        &load_script_body(conn, task_id).unwrap_or_default(),
    );

    let script_fragment = extract_script_fragment(&paragraphs, &unit);
    let assets_json = load_assets_json(conn, task_id);

    let relevant: Vec<&v5_parser::ParagraphIndexItem> = paragraphs
        .iter()
        .filter(|p| unit.section_refs.contains(&p.id))
        .collect();

    let context_params = json!({
        "unit": unit,
        "scriptFragment": script_fragment,
        "analysisContext": {
            "structureType": analysis.structure_type,
            "emotionMap": analysis.emotion_map,
            "relevantParagraphs": relevant,
        },
        "assetsJson": assets_json,
        "unitIndex": unit_index,
        "plannedEntryState": if plan_entry.is_empty() { serde_json::Value::Null } else { json!(plan_entry) },
        "previousPlanExit": if previous_plan_exit.is_empty() { serde_json::Value::Null } else { json!(previous_plan_exit) },
    });

    let full_text = match server_proxy::request_contextual_llm_stream(
        ContextualLlmParams {
            runtime_config: RuntimeConfig { ..runtime_config.clone() },
            context_type: "seedance_unit_efg".into(),
            context_params,
            temperature: Some(0.3),
            max_tokens_override: None,
        },
        |_chunk| {},
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            crate::services::seedance_store::upsert_unit(
                conn,
                task_id,
                unit_index as i32,
                Some(unit.duration_sec as i32),
                &unit.scene_type,
                Some(unit.sub_shot_count as i32),
                "",
                &NoteArea::default(),
                "failed",
                retry_count + 1,
                Some(&e),
            );
            return Err(e);
        }
    };

    if full_text.trim().is_empty() {
        let msg = "LLM 返回空, 请重试".to_string();
        crate::services::seedance_store::upsert_unit(
            conn,
            task_id,
            unit_index as i32,
            Some(unit.duration_sec as i32),
            &unit.scene_type,
            Some(unit.sub_shot_count as i32),
            "",
            &NoteArea::default(),
            "failed",
            retry_count + 1,
            Some(&msg),
        );
        return Err(msg);
    }

    let dual_region = v5_parser::parse_dual_region(&full_text);
    let copy_area = dual_region.copy_area;
    let note_area = dual_region.note_area;

    if copy_area.len() < 200 {
        let preview = if full_text.len() > 500 { &full_text[..500] } else { &full_text };
        let msg = format!(
            "LLM 产出异常: COPY 区字数 {} < 200. 原始前 500 字:\n{}",
            copy_area.len(),
            preview
        );
        crate::services::seedance_store::upsert_unit(
            conn,
            task_id,
            unit_index as i32,
            Some(unit.duration_sec as i32),
            &unit.scene_type,
            Some(unit.sub_shot_count as i32),
            &copy_area,
            &note_area,
            "failed",
            retry_count + 1,
            Some(&msg),
        );
        return Err(msg);
    }

    crate::services::seedance_store::upsert_unit(
        conn,
        task_id,
        unit_index as i32,
        Some(unit.duration_sec as i32),
        &unit.scene_type,
        Some(unit.sub_shot_count as i32),
        &copy_area,
        &note_area,
        "done",
        retry_count,
        None,
    );

    Ok(json!({
        "taskId": task_id,
        "unitIndex": unit_index,
        "durationSec": unit.duration_sec,
        "sceneType": unit.scene_type,
        "subShotCount": unit.sub_shot_count,
        "copyArea": copy_area,
        "noteArea": note_area,
        "status": "done",
        "retryCount": retry_count,
    }))
}

// ═══════════════════════════════════════════════════════════════
// Batch generation
// ═══════════════════════════════════════════════════════════════

pub async fn run_generate_all(
    conn: &Connection,
    task_id: &str,
    concurrency: Option<usize>,
) -> Result<Vec<serde_json::Value>, String> {
    let analysis = crate::services::seedance_store::load_analysis(conn, task_id)
        .ok_or_else(|| "请先生成 Phase A-D 分析.".to_string())?;

    let units = analysis.units.clone();
    let total = units.len();
    let _effective = concurrency.unwrap_or(1).min(12).max(1).min(total);

    let mut results = Vec::new();
    for u in &units {
        match run_unit_generation(conn, task_id, u.index).await {
            Ok(rec) => results.push(rec),
            Err(_) => {}
        }
    }

    results.sort_by_key(|r| r["unitIndex"].as_i64().unwrap_or(0));
    Ok(results)
}

pub fn list_all_units(conn: &Connection, task_id: &str) -> Vec<serde_json::Value> {
    crate::services::seedance_store::list_units(conn, task_id)
}

pub fn get_unit_record(conn: &Connection, task_id: &str, unit_index: i32) -> Option<serde_json::Value> {
    crate::services::seedance_store::get_unit(conn, task_id, unit_index)
}
