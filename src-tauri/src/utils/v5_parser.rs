use serde::Deserialize;
use serde::Serialize;

// ── Data types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParagraphFact {
    pub id: String,
    pub facts: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peak {
    pub section_id: String,
    pub kind: String,
    pub original_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Buffer {
    pub section_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subtext {
    pub section_id: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EmotionMap {
    pub peaks: Vec<Peak>,
    pub buffers: Vec<Buffer>,
    pub subtexts: Vec<Subtext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V5UnitPlan {
    pub index: usize,
    pub section_refs: Vec<String>,
    pub duration_sec: usize,
    pub scene_type: String,
    pub sub_shot_count: usize,
    pub summary: String,
    pub planned_entry_state: String,
    pub planned_exit_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct V5Analysis {
    pub paragraph_facts: Vec<ParagraphFact>,
    pub structure_type: String,
    pub emotion_map: EmotionMap,
    pub units: Vec<V5UnitPlan>,
    pub total_sec: usize,
    pub total_units: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParagraphIndexItem {
    pub id: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facts: Option<serde_json::Value>,
}

// ── Block parsing from markdown ──

#[derive(Debug)]
struct TypedBlock {
    type_name: String,
    id: String,
    fields: std::collections::HashMap<String, String>,
}

/// Split markdown by `## TYPE [ID]` headers into typed blocks.
fn parse_typed_blocks(markdown: &str) -> Vec<TypedBlock> {
    // Split on lines starting with ## (lookahead via positions)
    let re = regex_lite::Regex::new(r"(?m)^(##+\s+\S+)").unwrap();
    let mut headers: Vec<(usize, String)> = Vec::new();
    for cap in re.find_iter(markdown) {
        headers.push((cap.start(), cap.as_str().to_string()));
    }

    let mut blocks = Vec::new();
    for i in 0..headers.len() {
        let start = headers[i].0;
        let header_line = &headers[i].1;
        let end = if i + 1 < headers.len() {
            headers[i + 1].0
        } else {
            markdown.len()
        };

        let body = markdown[start + header_line.len()..end].trim();

        // Parse header: "## TYPE [ID]"
        let header_trimmed = header_line.trim_start_matches('#').trim();
        let (type_name, id) = if let Some(space) = header_trimmed.find(char::is_whitespace) {
            let tn = header_trimmed[..space].trim();
            let rest = header_trimmed[space..].trim();
            if rest.is_empty() {
                (tn.to_string(), String::new())
            } else {
                (tn.to_string(), rest.to_string())
            }
        } else {
            (header_trimmed.to_string(), String::new())
        };

        // Parse KV fields from body
        let mut fields = std::collections::HashMap::new();
        for line in body.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("---") {
                continue;
            }
            // Match "key: value" or "key：value"
            if let Some(idx) = trimmed.find(|c: char| c == ':' || c == '：') {
                let key = trimmed[..idx].trim();
                let value = trimmed[idx + 1..].trim();
                if !key.is_empty() {
                    fields.insert(key.to_string(), value.to_string());
                }
            }
        }

        blocks.push(TypedBlock {
            type_name,
            id,
            fields,
        });
    }

    blocks
}

// ── Number parsing ──

fn parse_int_safe(s: &str, fallback: usize) -> usize {
    s.trim().parse::<usize>().ok().unwrap_or(fallback)
}

fn parse_section_refs(s: &str) -> Vec<String> {
    if s.is_empty() {
        return Vec::new();
    }
    s.split(|c: char| c == ',' || c == '，' || c == ';' || c == '；' || c.is_whitespace())
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn normalize_scene_type(s: &str) -> String {
    match s.trim() {
        "文戏" | "快节奏文戏" | "武戏" | "环境戏" | "动作非武戏" => s.trim().to_string(),
        _ => "文戏".to_string(),
    }
}

// ── Field name mapping ──

const PARA_FIELD_MAP: &[(&str, &str)] = &[
    ("人物", "character"),
    ("character", "character"),
    ("动作", "action"),
    ("action", "action"),
    ("台词", "dialogue"),
    ("dialogue", "dialogue"),
    ("道具", "prop"),
    ("prop", "prop"),
    ("场景", "scene"),
    ("scene", "scene"),
    ("情绪", "emotion"),
    ("emotion", "emotion"),
];

fn map_para_field(key: &str, value: &str) -> Option<(String, serde_json::Value)> {
    for (k, mapped) in PARA_FIELD_MAP {
        if key.eq_ignore_ascii_case(k) {
            return Some((mapped.to_string(), serde_json::Value::String(value.to_string())));
        }
    }
    None
}

// ── Main parse function ──

pub fn parse_v5_analysis(markdown: &str) -> V5Analysis {
    let blocks = parse_typed_blocks(markdown);
    let mut warnings: Vec<String> = Vec::new();

    let meta_blocks: Vec<&TypedBlock> = blocks.iter().filter(|b| b.type_name == "META").collect();
    let para_blocks: Vec<&TypedBlock> = blocks.iter().filter(|b| b.type_name == "PARA").collect();
    let peak_blocks: Vec<&TypedBlock> = blocks.iter().filter(|b| b.type_name == "PEAK").collect();
    let buffer_blocks: Vec<&TypedBlock> = blocks.iter().filter(|b| b.type_name == "BUFFER").collect();
    let subtext_blocks: Vec<&TypedBlock> = blocks.iter().filter(|b| b.type_name == "SUBTEXT").collect();
    let unit_blocks: Vec<&TypedBlock> = blocks.iter().filter(|b| b.type_name == "UNIT").collect();

    // META
    let meta = meta_blocks.first().map(|b| &b.fields).cloned().unwrap_or_default();
    let structure_type = meta
        .get("structureType")
        .or_else(|| meta.get("结构类型"))
        .cloned()
        .unwrap_or_else(|| "linear".to_string());
    let total_sec = meta
        .get("totalSec")
        .or_else(|| meta.get("总时长"))
        .map(|s| parse_int_safe(s, 0))
        .unwrap_or(0);
    let total_units = meta
        .get("totalUnits")
        .or_else(|| meta.get("总单元数"))
        .map(|s| parse_int_safe(s, unit_blocks.len()))
        .unwrap_or(0);

    // PARA facts
    let paragraph_facts: Vec<ParagraphFact> = para_blocks
        .iter()
        .map(|b| {
            let mut facts = serde_json::Map::new();
            for (k, v) in &b.fields {
                if let Some((mapped, val)) = map_para_field(k, v) {
                    facts.insert(mapped, val);
                }
            }
            ParagraphFact {
                id: b.id.clone(),
                facts: serde_json::Value::Object(facts),
            }
        })
        .collect();

    // EMOTION
    let peaks: Vec<Peak> = peak_blocks
        .iter()
        .map(|b| Peak {
            section_id: b.id.clone(),
            kind: b.fields.get("kind").or_else(|| b.fields.get("类型")).cloned().unwrap_or_else(|| "其他".to_string()),
            original_ref: b.fields.get("originalRef").or_else(|| b.fields.get("原文引用")).cloned().unwrap_or_default(),
        })
        .collect();

    let buffers: Vec<Buffer> = buffer_blocks
        .iter()
        .map(|b| Buffer {
            section_id: b.id.clone(),
            reason: b.fields.get("reason").or_else(|| b.fields.get("原因")).cloned().unwrap_or_default(),
        })
        .collect();

    let subtexts: Vec<Subtext> = subtext_blocks
        .iter()
        .map(|b| Subtext {
            section_id: b.id.clone(),
            description: b.fields.get("description").or_else(|| b.fields.get("描述")).cloned().unwrap_or_default(),
        })
        .collect();

    // UNITS
    let units: Vec<V5UnitPlan> = unit_blocks
        .iter()
        .enumerate()
        .map(|(array_idx, b)| {
            let index = parse_int_safe(&b.id, array_idx + 1);
            let scene_id_raw = b.fields.get("sceneId").or_else(|| b.fields.get("场号"));
            let scene_id = scene_id_raw.and_then(|s| s.parse::<usize>().ok());
            V5UnitPlan {
                index,
                section_refs: parse_section_refs(b.fields.get("sectionRefs").or_else(|| b.fields.get("段号引用")).map(|s| s.as_str()).unwrap_or("")),
                duration_sec: parse_int_safe(b.fields.get("durationSec").or_else(|| b.fields.get("时长秒")).map(|s| s.as_str()).unwrap_or(""), 13),
                scene_type: normalize_scene_type(b.fields.get("sceneType").or_else(|| b.fields.get("场景类型")).map(|s| s.as_str()).unwrap_or("")),
                sub_shot_count: parse_int_safe(b.fields.get("subShotCount").or_else(|| b.fields.get("分镜数")).map(|s| s.as_str()).unwrap_or(""), 3),
                summary: b.fields.get("summary").or_else(|| b.fields.get("摘要")).cloned().unwrap_or_default(),
                planned_entry_state: b.fields.get("plannedEntryState").or_else(|| b.fields.get("起幅锚点")).cloned().unwrap_or_default(),
                planned_exit_state: b.fields.get("plannedExitState").or_else(|| b.fields.get("落幅锚点")).cloned().unwrap_or_default(),
                scene_id,
            }
        })
        .collect();

    if meta_blocks.is_empty() {
        warnings.push("META block 缺失 · 用默认值".to_string());
    }
    if unit_blocks.is_empty() {
        warnings.push("0 UNIT blocks · 异常".to_string());
    }
    if para_blocks.is_empty() {
        warnings.push("0 PARA blocks · 异常 (段落标注为空)".to_string());
    }
    if units.len() > 0 && total_units > 0 && total_units != units.len() {
        warnings.push(format!("totalUnits={} 但实际 {} 个 UNIT · 用实际数", total_units, units.len()));
    }
    let resolved_total_units = if units.is_empty() { total_units } else { units.len() };

    V5Analysis {
        paragraph_facts,
        structure_type,
        emotion_map: EmotionMap { peaks, buffers, subtexts },
        units,
        total_sec,
        total_units: resolved_total_units,
        warnings,
    }
}

/// Merge LLM-returned facts with the authoritative paragraphIndex text from the caller.
pub fn merge_v5_analysis(parsed: V5Analysis, server_paragraph_index: Vec<ParagraphIndexItem>) -> V5Analysis {
    let facts_map: std::collections::HashMap<String, serde_json::Value> = parsed
        .paragraph_facts
        .iter()
        .map(|pf| (pf.id.clone(), pf.facts.clone()))
        .collect();

    let _merged_paragraph_index: Vec<ParagraphIndexItem> = server_paragraph_index
        .into_iter()
        .map(|p| ParagraphIndexItem {
            facts: Some(facts_map.get(&p.id).cloned().unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()))),
            ..p
        })
        .collect();

    // Return as plain JSON for serialization flexibility
    V5Analysis {
        paragraph_facts: parsed.paragraph_facts,
        ..parsed
    }
}

// ── Dual region parsing (Phase E-F-G output) ──

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NoteArea {
    pub traceback: String,
    pub self_check_report: std::collections::HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_unit_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DualRegion {
    pub copy_area: String,
    pub note_area: NoteArea,
}

/// Parse V5 F1 dual-region markdown:
///   ═══ COPY 区 START ═══
///   ...
///   ═══ COPY 区 END · NOTE 区 START ═══
///   ...
///   ═══ NOTE 区 END ═══
pub fn parse_dual_region(text: &str) -> DualRegion {
    // Loose matching for various separator formats
    let copy_start_re = regex_lite::Regex::new(r"(?i)═{3,}.*?📋?\s*COPY\s*区\s*START").unwrap();
    let copy_end_re = regex_lite::Regex::new(r"(?i)═{3,}.*?📋?\s*COPY\s*区\s*END").unwrap();
    let note_start_re = regex_lite::Regex::new(r"(?i)═{3,}.*?📝?\s*NOTE\s*区\s*START").unwrap();
    let note_end_re = regex_lite::Regex::new(r"(?i)═{3,}.*?📝?\s*NOTE\s*区\s*END").unwrap();

    let copy_start = copy_start_re.find(text).map(|m| m.start());
    let copy_end = copy_end_re.find(text).map(|m| m.start());
    let note_start = note_start_re.find(text).map(|m| m.start());
    let note_end = note_end_re.find(text).map(|m| m.start());

    let mut copy_area = String::new();
    let mut note_raw = String::new();

    if let (Some(cs), Some(ce)) = (copy_start, copy_end) {
        if ce > cs {
            let after_start = text[cs..].find('\n').unwrap_or(0);
            copy_area = text[cs + after_start + 1..ce].trim().to_string();
            let sep_re = regex_lite::Regex::new(r"(?m)^═+\s*$").unwrap();
            copy_area = sep_re.replace_all(&copy_area, "").trim().to_string();
        }
    } else {
        copy_area = text.trim().to_string();
    }

    if let (Some(ns), Some(ne)) = (note_start, note_end) {
        if ne > ns {
            let after_start = text[ns..].find('\n').unwrap_or(0);
            note_raw = text[ns + after_start + 1..ne].trim().to_string();
            let sep_re = regex_lite::Regex::new(r"(?m)^═+\s*$").unwrap();
            note_raw = sep_re.replace_all(&note_raw, "").trim().to_string();
        }
    }

    // Parse NOTE area
    let mut self_check_report: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut in_traceback = false;
    let mut in_next_hint = false;
    let mut traceback_lines: Vec<String> = Vec::new();
    let mut next_hint_lines: Vec<String> = Vec::new();

    let traceback_header_re = regex_lite::Regex::new(r"(?i)^##\s*(段号溯源|溯源)").unwrap();
    let next_hint_header_re = regex_lite::Regex::new(r"(?i)^##\s*(下一单元衔接参考|衔接参考|nextUnitHint|next)").unwrap();
    let selfcheck_header_re = regex_lite::Regex::new(r"(?i)^##\s*(自检报告|自检)").unwrap();
    let selfcheck_item_re = regex_lite::Regex::new(r"^(G-?\d+(?:\.\d+)?)[:\s：]+(.+)$").unwrap();

    for line in note_raw.lines() {
        let trimmed = line.trim();
        if traceback_header_re.is_match(trimmed) {
            in_traceback = true;
            in_next_hint = false;
            continue;
        }
        if next_hint_header_re.is_match(trimmed) {
            in_next_hint = true;
            in_traceback = false;
            continue;
        }
        if selfcheck_header_re.is_match(trimmed) {
            in_traceback = false;
            in_next_hint = false;
            continue;
        }

        if let Some(cap) = selfcheck_item_re.captures(trimmed) {
            let key = cap.get(1).unwrap().as_str().replace(' ', "");
            let val = cap.get(2).unwrap().as_str().trim().to_string();
            let status = if val.contains('❌') || val.contains("fail") || val.contains("失败") || val.contains("违反") {
                "fail"
            } else if val.contains('⚠') || val.contains("warn") || val.contains("警告") {
                "warn"
            } else {
                "pass"
            };
            self_check_report.insert(key, status.to_string());
            continue;
        }

        if in_traceback && !trimmed.is_empty() {
            traceback_lines.push(trimmed.to_string());
        }
        if in_next_hint && !trimmed.is_empty() {
            next_hint_lines.push(trimmed.to_string());
        }
    }

    let traceback = traceback_lines.join(" · ");
    let next_unit_hint = next_hint_lines.join(" · ");

    DualRegion {
        copy_area,
        note_area: NoteArea {
            traceback,
            self_check_report,
            next_unit_hint: if next_unit_hint.is_empty() { None } else { Some(next_unit_hint) },
        },
    }
}
