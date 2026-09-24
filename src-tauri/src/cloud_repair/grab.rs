//! 受限抓取：模型**主动**从本地取自己需要的内容。
//!
//! 校核包只带「这部分题目需要的」上下文。模型认为还不够时，有两条路：
//!
//! 1. `report_insufficient_context` —— 说清缺什么，由后端在预算内替它取；
//! 2. 直接用这里的工具自己取（指定页、按原文搜索、页图区域、文章段落、候选切片）。
//!
//! 两条路都**只读**，且都受同一份预算约束（见 [`GrabBudget`]）。工具全部不接受路径：
//! 来源由后端按 job 解析，模型能选的只有「哪一页 / 哪段话 / 哪个区域」。这一条不是
//! 形式主义——`read_source` 以前允许不带页范围，于是它一次就能拿回整卷，包的存在
//! 就没有意义了。
//!
//! 页号约定与 [`super::packets`] 一致：对外一律 **1-based**。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::packets::{PageImageRef, SourceLine, SourcePageIndex, SourceParagraph};

/// 单次 `read_source` 最多返回几页。**这是硬上限**：它正是「包」这件事能成立的前提。
pub(crate) const MAX_READ_SOURCE_PAGES: u64 = 3;
/// 每包允许的抓取次数。
pub(crate) const PACKET_GRAB_LIMIT: u32 = 3;
/// 每包允许抓取的累计页数。
pub(crate) const PACKET_GRAB_PAGE_LIMIT: u64 = 6;
/// 每包抓取内容的累计字节上限。
pub(crate) const PACKET_GRAB_BYTE_LIMIT: usize = 160_000;
/// `search_source` 最多返回多少条命中。
pub(crate) const MAX_SEARCH_HITS: usize = 20;
/// `search_source` 每条命中前后各带几行。
const SEARCH_CONTEXT_LINES: usize = 2;

/// 一个包的抓取预算。**按包独立计数**，换包清零。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GrabBudget {
    pub calls: u32,
    pub pages: u64,
    pub bytes: usize,
}

impl Default for GrabBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl GrabBudget {
    pub fn new() -> Self {
        GrabBudget {
            calls: 0,
            pages: 0,
            bytes: 0,
        }
    }

    /// 记账。超预算时返回**具体**原因，模型要能据此缩小请求而不是收到一句「失败」。
    pub fn charge(&mut self, pages: u64, bytes: usize) -> Result<(), String> {
        if self.calls + 1 > PACKET_GRAB_LIMIT {
            return Err(format!(
                "CLOUD_GRAB_BUDGET_EXHAUSTED:calls:limit={PACKET_GRAB_LIMIT}:used={}",
                self.calls
            ));
        }
        if self.pages + pages > PACKET_GRAB_PAGE_LIMIT {
            return Err(format!(
                "CLOUD_GRAB_BUDGET_EXHAUSTED:pages:limit={PACKET_GRAB_PAGE_LIMIT}:used={}:requested={pages}",
                self.pages
            ));
        }
        if self.bytes + bytes > PACKET_GRAB_BYTE_LIMIT {
            return Err(format!(
                "CLOUD_GRAB_BUDGET_EXHAUSTED:bytes:limit={PACKET_GRAB_BYTE_LIMIT}:used={}",
                self.bytes
            ));
        }
        self.calls += 1;
        self.pages += pages;
        self.bytes += bytes;
        Ok(())
    }

    pub fn exhausted(&self) -> bool {
        self.calls >= PACKET_GRAB_LIMIT || self.pages >= PACKET_GRAB_PAGE_LIMIT
    }
}

/// 读好一份原文页索引：逐行文本、页图、答案页、段落。
///
/// 只读**解析器层**产物（`document-ir.json` / 比对报告 / 页图 / 答案页识别结果），
/// **绝不**读 `authoring-ir.json`：那是语义识别结论，正是要被纠正的东西。
pub(crate) fn load_source_index(
    root: &Path,
    job_id: &str,
    source_file_id: &str,
    kind: &str,
) -> SourcePageIndex {
    let dir = crate::util::job_dir(root, job_id);
    let mut index = SourcePageIndex {
        source_file_id: source_file_id.to_string(),
        kind: kind.to_string(),
        ..Default::default()
    };

    // ── 逐行文本（1-based 页号）─────────────────────────────────────────
    if let Ok(Some(ir)) = crate::util::read_json_opt(&dir.join("document-ir.json")) {
        for page in ir
            .get("pages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(zero_based) = page.get("pageIndex").and_then(Value::as_u64) else {
                continue;
            };
            let page_number = zero_based as u32 + 1;
            let mut lines: Vec<SourceLine> = Vec::new();
            if let Some(entries) = page.get("lines").and_then(Value::as_array) {
                for entry in entries {
                    let Some(text) = entry.get("text").and_then(Value::as_str) else {
                        continue;
                    };
                    if text.trim().is_empty() {
                        continue;
                    }
                    lines.push(SourceLine {
                        id: format!("p{page_number}:l{}", lines.len() + 1),
                        text: text.trim_end().to_string(),
                    });
                }
            } else if let Some(spans) = page.get("spans").and_then(Value::as_array) {
                for span in spans {
                    let Some(text) = span.get("text").and_then(Value::as_str) else {
                        continue;
                    };
                    if text.trim().is_empty() {
                        continue;
                    }
                    lines.push(SourceLine {
                        id: format!("p{page_number}:l{}", lines.len() + 1),
                        text: text.trim_end().to_string(),
                    });
                }
            }
            if !lines.is_empty() {
                index.lines.insert(page_number, lines);
            }
        }
    }
    if index.lines.is_empty() {
        if let Ok(Some(report)) =
            crate::util::read_json_opt(&dir.join("document-ir-v2.shadow.compare.json"))
        {
            for page in report
                .get("pages")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(zero_based) = page.get("pageIndex").and_then(Value::as_u64) else {
                    continue;
                };
                let page_number = zero_based as u32 + 1;
                let text = page
                    .get("v1Text")
                    .and_then(Value::as_str)
                    .or_else(|| page.get("v2Text").and_then(Value::as_str))
                    .unwrap_or("");
                let lines: Vec<SourceLine> = text
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .enumerate()
                    .map(|(position, line)| SourceLine {
                        id: format!("p{page_number}:l{}", position + 1),
                        text: line.to_string(),
                    })
                    .collect();
                if !lines.is_empty() {
                    index.lines.insert(page_number, lines);
                }
            }
        }
    }

    // ── 页图（1-based 页号，带点尺寸）───────────────────────────────────
    if let Ok(Some(extraction)) =
        crate::util::read_json_opt(&dir.join("cache").join("vision").join("pdf-images.json"))
    {
        for page in extraction
            .get("pages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(page_number) = page.get("pageIndex").and_then(Value::as_u64) else {
                continue;
            };
            let Some(image) = page
                .get("images")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|image| image.get("path").and_then(Value::as_str).is_some())
            else {
                continue;
            };
            let Some(path) = image.get("path").and_then(Value::as_str) else {
                continue;
            };
            index.page_images.insert(
                page_number as u32,
                PageImageRef {
                    path: path.to_string(),
                    mime_type: image
                        .get("mimeType")
                        .and_then(Value::as_str)
                        .unwrap_or("image/png")
                        .to_string(),
                    width: page.get("width").and_then(Value::as_f64).unwrap_or(595.0),
                    height: page.get("height").and_then(Value::as_f64).unwrap_or(842.0),
                },
            );
        }
    }

    // ── 答案页（识别产物优先；没有线索时保持 `known = false`）─────────────
    if let Ok(Some(candidates)) =
        crate::util::read_json_opt(&dir.join("vision-answer-candidates.json"))
    {
        let pages: Vec<u32> = candidates
            .get("answerPageIndexes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|page| page as u32)
            .collect();
        if !pages.is_empty() {
            index.answer_pages = pages;
            index.answer_pages_known = true;
        }
    }

    // ── 段落（非 PDF：逐行当作段落，id 用行 id）─────────────────────────
    if index.kind != "pdf" {
        index.paragraphs = index
            .lines
            .values()
            .flatten()
            .map(|line| SourceParagraph {
                id: line.id.clone(),
                text: line.text.clone(),
            })
            .collect();
    }

    index
}

/// `read_source`（**收紧版**）：必须给页范围或引文，单次最多 [`MAX_READ_SOURCE_PAGES`] 页。
pub(crate) fn read_source(
    source: &SourcePageIndex,
    arguments: &Value,
    budget: &mut GrabBudget,
) -> Result<Value, String> {
    let page_from = arguments.get("pageIndex").and_then(Value::as_u64);
    let page_to = arguments
        .get("pageTo")
        .and_then(Value::as_u64)
        .or(page_from);
    let quote = arguments
        .get("quote")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if page_from.is_none() && quote.is_none() {
        return Err(
            "CLOUD_GRAB_PAGE_RANGE_REQUIRED: pass {\"pageIndex\": N} (optionally \"pageTo\") \
             or a \"quote\" to locate. Reading the whole paper in one call is not allowed; \
             the packet already lists the pages that matter and scopeManifest says how to get more."
                .to_string(),
        );
    }

    let (from, to) = match page_from {
        Some(from) => {
            let to = page_to.unwrap_or(from);
            if to < from {
                return Err(format!(
                    "CLOUD_GRAB_PAGE_RANGE_INVALID:pageTo({to}) is before pageIndex({from})"
                ));
            }
            let count = to - from + 1;
            if count > MAX_READ_SOURCE_PAGES {
                return Err(format!(
                    "CLOUD_GRAB_PAGE_LIMIT_EXCEEDED:max={MAX_READ_SOURCE_PAGES}:requested={count}"
                ));
            }
            (from, to)
        }
        None => {
            // 只给了引文：先在文本层里定位，再取那一页。
            let needle = quote.unwrap_or_default();
            let Some((page, _)) = find_line(source, needle) else {
                return Err(format!(
                    "CLOUD_GRAB_QUOTE_NOT_FOUND: no line in the extracted text layer contains \
                     {needle:?}. Try search_source with a shorter phrase."
                ));
            };
            (u64::from(page), u64::from(page))
        }
    };

    let available: Vec<u32> = source.lines.keys().copied().collect();
    for page in from..=to {
        let page_number = page as u32;
        let known = source.lines.contains_key(&page_number)
            || source.page_images.contains_key(&page_number);
        if !known {
            let range = match (available.first(), available.last()) {
                (Some(low), Some(high)) => format!("{low}-{high}"),
                _ => "none".to_string(),
            };
            return Err(format!(
                "CLOUD_GRAB_PAGE_OUT_OF_RANGE:page={page_number}:available={range}"
            ));
        }
    }

    let pages: Vec<Value> = (from..=to)
        .map(|page| {
            let page_number = page as u32;
            let lines: Vec<Value> = source
                .lines
                .get(&page_number)
                .map(|lines| {
                    lines
                        .iter()
                        .map(|line| json!({"id": line.id, "text": line.text}))
                        .collect()
                })
                .unwrap_or_default();
            json!({
                "pageIndex": page_number,
                "lines": lines,
                "hasPageImage": source.page_images.contains_key(&page_number),
            })
        })
        .collect();
    let payload = json!({
        "kind": source.kind,
        "sourceFileId": source.source_file_id,
        "note": "These are the extracted text-layer lines. Cite them verbatim with their line id and page number.",
        "pages": pages,
    });
    let bytes = serde_json::to_string(&payload).map(|text| text.len()).unwrap_or(0);
    budget.charge(to - from + 1, bytes)?;
    Ok(payload)
}

/// `search_source`：只记得一句话、不知道在哪一页时用。
pub(crate) fn search_source(
    source: &SourcePageIndex,
    arguments: &Value,
    budget: &mut GrabBudget,
) -> Result<Value, String> {
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "CLOUD_GRAB_QUERY_REQUIRED: pass {\"query\": \"a phrase you remember from the file\"}"
                .to_string()
        })?;
    let needle = normalize(query);
    let mut hits: Vec<Value> = Vec::new();
    for (page, lines) in &source.lines {
        for (position, line) in lines.iter().enumerate() {
            if !normalize(&line.text).contains(&needle) {
                continue;
            }
            let before: Vec<Value> = lines[position.saturating_sub(SEARCH_CONTEXT_LINES)..position]
                .iter()
                .map(|line| json!({"id": line.id, "text": line.text}))
                .collect();
            let after: Vec<Value> = lines
                [position + 1..(position + 1 + SEARCH_CONTEXT_LINES).min(lines.len())]
                .iter()
                .map(|line| json!({"id": line.id, "text": line.text}))
                .collect();
            hits.push(json!({
                "pageIndex": page,
                "lineId": line.id,
                "text": line.text,
                "before": before,
                "after": after,
            }));
            if hits.len() >= MAX_SEARCH_HITS {
                break;
            }
        }
        if hits.len() >= MAX_SEARCH_HITS {
            break;
        }
    }
    let payload = json!({
        "kind": source.kind,
        "query": query,
        "hits": hits,
        "note": if hits.is_empty() {
            "No line in the extracted text layer contains this phrase. Try a shorter or differently worded phrase, or read_source with a page range."
        } else {
            "Copy the line text verbatim into your evidence, with its lineId and pageIndex."
        },
    });
    let bytes = serde_json::to_string(&payload).map(|text| text.len()).unwrap_or(0);
    budget.charge(0, bytes)?;
    Ok(payload)
}

/// `read_page_region`：一页图，可裁剪到一个 bbox。
pub(crate) fn read_page_region(
    root: &Path,
    job_id: &str,
    source: &SourcePageIndex,
    arguments: &Value,
    budget: &mut GrabBudget,
) -> Result<Value, String> {
    let page = arguments
        .get("pageIndex")
        .and_then(Value::as_u64)
        .filter(|page| *page >= 1)
        .ok_or_else(|| {
            "CLOUD_GRAB_PAGE_REQUIRED: pass {\"pageIndex\": N} (1-based) and optionally a bbox"
                .to_string()
        })? as u32;
    let Some(image) = source.page_images.get(&page) else {
        let available: Vec<u32> = source.page_images.keys().copied().collect();
        return Err(format!(
            "CLOUD_GRAB_PAGE_IMAGE_UNAVAILABLE:page={page}:pages_with_images={available:?}"
        ));
    };
    let bbox = arguments.get("bbox").filter(|bbox| !bbox.is_null()).cloned();
    let cropped = crop_page_image(root, job_id, "grab", page, image, bbox.as_ref())?;
    let payload = json!({
        "pageIndex": page,
        "cropped": cropped.is_some(),
        "image": {
            "path": cropped.clone().unwrap_or_else(|| image.path.clone()),
            "mimeType": if cropped.is_some() { "image/png" } else { image.mime_type.as_str() },
            "pageWidth": image.width,
            "pageHeight": image.height,
        },
    });
    let bytes = serde_json::to_string(&payload).map(|text| text.len()).unwrap_or(0);
    budget.charge(0, bytes)?;
    Ok(payload)
}

/// 把包里的区域请求裁成真实图片，写进 `<job>/cache/repair-packets/`。
///
/// 裁剪失败**不**让整包失败：退回整页图（任务书允许「锚点缺失时改用整页图」），
/// 并在返回里如实说明退回了哪几张——静默退回会让模型以为它看到的是一块区域图。
pub(crate) fn materialize_regions(
    root: &Path,
    job_id: &str,
    packet_id: &str,
    source: &SourcePageIndex,
    regions: &[Value],
) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut seen: BTreeMap<(u32, String), usize> = BTreeMap::new();
    for region in regions {
        let Some(page) = region.get("pageIndex").and_then(Value::as_u64) else {
            continue;
        };
        let page = page as u32;
        let bbox = region.get("bbox").filter(|bbox| !bbox.is_null()).cloned();
        let key = (
            page,
            serde_json::to_string(&bbox).unwrap_or_default(),
        );
        if let Some(existing) = seen.get(&key) {
            let mut merged = out[*existing].clone();
            if let Some(task_id) = region.get("taskId") {
                if let Some(list) = merged.get_mut("taskIds").and_then(Value::as_array_mut) {
                    list.push(task_id.clone());
                }
            }
            out[*existing] = merged;
            continue;
        }
        let mut entry = region.clone();
        let mut cropped = None;
        let mut note = Value::Null;
        match source.page_images.get(&page) {
            Some(image) => match crop_page_image(root, job_id, packet_id, page, image, bbox.as_ref()) {
                Ok(path) => {
                    if let Some(path) = path {
                        cropped = Some(path);
                    } else {
                        note = json!("full page image (this page has no crop, or the crop failed)");
                    }
                }
                Err(error) => {
                    note = json!(error);
                }
            },
            None => {
                note = json!("no page image available for this page");
            }
        }
        let image = match (&cropped, source.page_images.get(&page)) {
            (Some(path), _) => json!({"path": path, "mimeType": "image/png"}),
            (None, Some(image)) => json!({"path": image.path, "mimeType": image.mime_type}),
            (None, None) => Value::Null,
        };
        entry["image"] = image;
        entry["taskIds"] = json!(region
            .get("taskId")
            .cloned()
            .into_iter()
            .collect::<Vec<_>>());
        if let Some(object) = entry.as_object_mut() {
            object.remove("taskId");
            object.insert("note".to_string(), note);
        }
        seen.insert(key, out.len());
        out.push(entry);
    }
    out
}

/// 裁剪一页图。返回 `None` 表示「没有裁剪」（无 bbox、图不是 PNG、或解码失败），
/// 调用方据此退回整页图。
fn crop_page_image(
    root: &Path,
    job_id: &str,
    packet_id: &str,
    page: u32,
    image: &PageImageRef,
    bbox: Option<&Value>,
) -> Result<Option<String>, String> {
    let Some(bbox) = bbox else {
        return Ok(None);
    };
    let (Some(x), Some(y), Some(width), Some(height)) = (
        bbox.get("x").and_then(Value::as_f64),
        bbox.get("y").and_then(Value::as_f64),
        bbox.get("width").and_then(Value::as_f64),
        bbox.get("height").and_then(Value::as_f64),
    ) else {
        return Ok(None);
    };
    if width <= 0.0 || height <= 0.0 || image.width <= 0.0 || image.height <= 0.0 {
        return Ok(None);
    }
    // 锚点 bbox 的原点可能是左下；裁剪要的是从上往下。
    let bottom_left = bbox.get("origin").and_then(Value::as_str) == Some("bottom-left");
    let top = if bottom_left { y + height } else { y };
    let margin_x = width * 0.05;
    let margin_y = height * 0.10;

    let bytes = std::fs::read(&image.path)
        .map_err(|error| format!("CLOUD_GRAB_REGION_READ_FAILED:{}:{error}", image.path))?;
    let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
    let mut reader = match decoder.read_info() {
        Ok(reader) => reader,
        // 不是 PNG（例如扫描页的 JPEG）：如实退回整页图，不假装裁过。
        Err(_) => return Ok(None),
    };
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| format!("CLOUD_GRAB_REGION_DECODE_FAILED:{error}"))?;
    let pixel_width = info.width;
    let pixel_height = info.height;
    let scale_x = pixel_width as f64 / image.width;
    let scale_y = pixel_height as f64 / image.height;
    let left = ((x - margin_x) * scale_x).max(0.0).min(pixel_width as f64 - 1.0);
    let right = ((x + width + margin_x) * scale_x)
        .max(left + 1.0)
        .min(pixel_width as f64);
    let top = ((top - height - margin_y) * scale_y).max(0.0).min(pixel_height as f64 - 1.0);
    let bottom = ((top + height * 3.0 + margin_y) * scale_y)
        .max(top + 1.0)
        .min(pixel_height as f64);
    let left = left as u32;
    let top = top as u32;
    let crop_width = (right as u32).saturating_sub(left).max(1);
    let crop_height = (bottom as u32).saturating_sub(top).max(1);
    let channels = match info.color_type {
        png::ColorType::Rgb => 3usize,
        png::ColorType::Rgba => 4usize,
        png::ColorType::Grayscale => 1usize,
        png::ColorType::GrayscaleAlpha => 2usize,
        png::ColorType::Indexed => return Ok(None),
    };

    let mut out = Vec::with_capacity((crop_width * crop_height) as usize * channels);
    for row in top..(top + crop_height).min(pixel_height) {
        let start = ((row * pixel_width + left) as usize) * channels;
        let end = start + (crop_width as usize) * channels;
        if end > buffer.len() {
            break;
        }
        out.extend_from_slice(&buffer[start..end]);
    }
    let directory = crate::util::job_dir(root, job_id)
        .join("cache")
        .join("repair-packets");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("CLOUD_GRAB_REGION_DIR_FAILED:{error}"))?;
    let file_name = format!(
        "{packet_id}-p{page}-{left}-{top}-{crop_width}x{crop_height}.png"
    );
    let out_path: PathBuf = directory.join(file_name);
    let file = std::fs::File::create(&out_path)
        .map_err(|error| format!("CLOUD_GRAB_REGION_WRITE_FAILED:{error}"))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), crop_width, crop_height);
    encoder.set_color(match channels {
        3 => png::ColorType::Rgb,
        4 => png::ColorType::Rgba,
        1 => png::ColorType::Grayscale,
        _ => png::ColorType::GrayscaleAlpha,
    });
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("CLOUD_GRAB_REGION_ENCODE_FAILED:{error}"))?;
    writer
        .write_image_data(&out)
        .map_err(|error| format!("CLOUD_GRAB_REGION_ENCODE_FAILED:{error}"))?;
    Ok(Some(out_path.to_string_lossy().to_string()))
}

/// `read_passage`：按段落标号或题号取文章段落。
///
/// 文章正文**从不整段进包**（它是整卷里最大的一块），所以这个工具是模型读到正文的
/// 唯一途径——也正因如此，它必须真的能按题号定位，而不是只能返回开头一段。
pub(crate) fn read_passage(
    source: &SourcePageIndex,
    arguments: &Value,
    budget: &mut GrabBudget,
) -> Result<Value, String> {
    let labels: Vec<String> = arguments
        .get("paragraphLabels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|label| label.trim().to_ascii_uppercase())
        .filter(|label| !label.is_empty())
        .collect();
    let numbers: Vec<u32> = arguments
        .get("questionNumbers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .map(|number| number as u32)
        .collect();
    if labels.is_empty() && numbers.is_empty() {
        return Err(
            "CLOUD_GRAB_PASSAGE_SELECTOR_REQUIRED: pass {\"paragraphLabels\": [\"C\",\"D\"]} \
             or {\"questionNumbers\": [14,15]}. The whole passage is never sent at once."
                .to_string(),
        );
    }

    let mut paragraphs: Vec<Value> = Vec::new();
    let mut anchors: Vec<u32> = Vec::new();
    for (page, lines) in &source.lines {
        for (position, line) in lines.iter().enumerate() {
            let trimmed = line.text.trim_start();
            let matches_label = labels.iter().any(|label| {
                // 段落标号形如 `C` / `C.` / `Paragraph C`，行首且后面跟内容。
                let upper = trimmed.to_ascii_uppercase();
                let head: String = upper.chars().take_while(|ch| ch.is_ascii_alphanumeric()).collect();
                head == *label || head == format!("{label}.")
            });
            let matches_number = numbers.iter().any(|number| {
                let digits: String = trimmed.chars().take_while(char::is_ascii_digit).collect();
                digits.parse::<u32>().map(|value| value == *number).unwrap_or(false)
            });
            if !matches_label && !matches_number {
                continue;
            }
            anchors.push(*page);
            paragraphs.push(json!({
                "pageIndex": page,
                "lineId": line.id,
                "text": line.text,
                "matchedBy": if matches_label { "label" } else { "questionNumber" },
                "window": lines[position.saturating_sub(1)..(position + 4).min(lines.len())]
                    .iter()
                    .map(|line| json!({"id": line.id, "text": line.text}))
                    .collect::<Vec<_>>(),
            }));
            if paragraphs.len() >= MAX_SEARCH_HITS {
                break;
            }
        }
    }
    anchors.sort_unstable();
    anchors.dedup();
    let payload = json!({
        "paragraphs": paragraphs,
        "pages": anchors,
        "note": if paragraphs.is_empty() {
            "Nothing matched those labels or question numbers in the extracted text layer. Try search_source."
        } else {
            "The window around each match is the passage text you may cite."
        },
    });
    let bytes = serde_json::to_string(&payload).map(|text| text.len()).unwrap_or(0);
    budget.charge(anchors.len() as u64, bytes)?;
    Ok(payload)
}

/// 在一行文本里找 needle（规范化空白与大小写后比较）。
fn find_line(source: &SourcePageIndex, needle: &str) -> Option<(u32, String)> {
    let wanted = normalize(needle);
    if wanted.is_empty() {
        return None;
    }
    for (page, lines) in &source.lines {
        for line in lines {
            if normalize(&line.text).contains(&wanted) {
                return Some((*page, line.id.clone()));
            }
        }
    }
    None
}

fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// 把一次 `report_insufficient_context` 的需求清单翻译成**具体**的抓取动作。
///
/// 返回三份：
/// - `satisfied`：取到的证据（下一轮并入本包）；
/// - `unsatisfied`：没取到的需求与**具体原因**。必须如实回给模型——静默丢掉一条需求，
///   模型下一轮会以为它已经被满足了；
/// - `deferred`：`candidate` / `draft` 两类需求，本模块读不到（它只有原文索引），
///   由编排层满足。它们**不**算「没取到」，所以不放进 `unsatisfied`。
pub(crate) fn satisfy_needs(
    root: &Path,
    job_id: &str,
    source: &SourcePageIndex,
    needs: &[Value],
    budget: &mut GrabBudget,
) -> (Vec<Value>, Vec<String>, Vec<Value>) {
    let mut satisfied: Vec<Value> = Vec::new();
    let mut unsatisfied: Vec<String> = Vec::new();
    let mut deferred: Vec<Value> = Vec::new();
    for need in needs {
        let kind = need.get("kind").and_then(Value::as_str).unwrap_or("");
        let result = match kind {
            "pages" => {
                let from = need.get("from").and_then(Value::as_u64);
                let to = need.get("to").and_then(Value::as_u64);
                match from {
                    Some(from) => read_source(
                        source,
                        &json!({"pageIndex": from, "pageTo": to.unwrap_or(from)}),
                        budget,
                    ),
                    None => Err("CLOUD_GRAB_NEED_MALFORMED:pages:missing \"from\"".to_string()),
                }
            }
            "search" => match need.get("quote").and_then(Value::as_str) {
                Some(quote) => search_source(source, &json!({"query": quote}), budget),
                None => Err("CLOUD_GRAB_NEED_MALFORMED:search:missing \"quote\"".to_string()),
            },
            "page_region" => {
                let mut arguments = json!({
                    "pageIndex": need.get("pageIndex").cloned().unwrap_or(Value::Null),
                });
                if let Some(bbox) = need.get("bbox").filter(|bbox| !bbox.is_null()) {
                    arguments["bbox"] = bbox.clone();
                }
                read_page_region(root, job_id, source, &arguments, budget)
            }
            "passage" => {
                let mut arguments = json!({});
                if let Some(labels) = need.get("paragraphLabels") {
                    arguments["paragraphLabels"] = labels.clone();
                }
                if let Some(numbers) = need.get("questionNumbers") {
                    arguments["questionNumbers"] = numbers.clone();
                }
                read_passage(source, &arguments, budget)
            }
            // 候选切片与稿件切片由编排层（那里才有 candidate 与 canonical）满足。
            "candidate" | "draft" => {
                deferred.push(need.clone());
                continue;
            }
            other => {
                unsatisfied.push(format!("CLOUD_GRAB_NEED_UNKNOWN_KIND:{other}"));
                continue;
            }
        };
        match result {
            Ok(value) => satisfied.push(json!({"kind": kind, "result": value})),
            Err(error) => unsatisfied.push(error),
        }
    }
    (satisfied, unsatisfied, deferred)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> SourcePageIndex {
        let mut source = SourcePageIndex {
            source_file_id: "src-1".to_string(),
            kind: "pdf".to_string(),
            answer_pages_known: false,
            ..Default::default()
        };
        for (page, lines) in [
            (1u32, vec!["A The passage begins here.", "1 A statement."]),
            (2, vec!["Questions 14-20", "14 First question"]),
            (3, vec!["Answers", "14 B", "15 D"]),
        ] {
            source.lines.insert(
                page,
                lines
                    .into_iter()
                    .enumerate()
                    .map(|(position, text)| SourceLine {
                        id: format!("p{page}:l{}", position + 1),
                        text: text.to_string(),
                    })
                    .collect(),
            );
            source.page_images.insert(
                page,
                PageImageRef {
                    path: format!("/tmp/does-not-exist-{page}.png"),
                    mime_type: "image/png".to_string(),
                    width: 595.0,
                    height: 842.0,
                },
            );
        }
        source
    }

    /// 不带页范围也不带引文的 `read_source` 必须被拒，且原因要具体。
    #[test]
    fn read_source_without_a_selector_is_rejected_with_a_specific_reason() {
        let mut budget = GrabBudget::new();
        let error = read_source(&source(), &json!({}), &mut budget).expect_err("必须被拒");
        assert!(
            error.starts_with("CLOUD_GRAB_PAGE_RANGE_REQUIRED"),
            "错误必须具体到「该给什么」：{error}"
        );
        assert_eq!(budget.calls, 0, "被拒的调用不该消耗预算");
    }

    /// 超出单次页数上限必须被拒。
    #[test]
    fn read_source_beyond_the_page_limit_is_rejected() {
        let mut budget = GrabBudget::new();
        let error = read_source(&source(), &json!({"pageIndex": 1, "pageTo": 4}), &mut budget)
            .expect_err("必须被拒");
        assert!(error.starts_with("CLOUD_GRAB_PAGE_LIMIT_EXCEEDED"), "{error}");
        let ok = read_source(&source(), &json!({"pageIndex": 1, "pageTo": 3}), &mut budget)
            .expect("三页以内允许");
        assert_eq!(ok["pages"].as_array().unwrap().len(), 3);
    }

    /// 越界页必须被拒，并告诉模型哪些页是有的。
    #[test]
    fn read_source_beyond_the_document_is_rejected() {
        let mut budget = GrabBudget::new();
        let error = read_source(&source(), &json!({"pageIndex": 99}), &mut budget)
            .expect_err("必须被拒");
        assert!(error.starts_with("CLOUD_GRAB_PAGE_OUT_OF_RANGE"), "{error}");
        assert!(error.contains("1-3"), "要告诉模型可用的页范围：{error}");
    }

    /// 预算用尽后继续抓取必须被拒，且理由带数字。
    #[test]
    fn the_grab_budget_stops_further_reads() {
        let mut budget = GrabBudget::new();
        for _ in 0..PACKET_GRAB_LIMIT {
            read_source(&source(), &json!({"pageIndex": 1}), &mut budget).expect("预算内");
        }
        let error = read_source(&source(), &json!({"pageIndex": 1}), &mut budget)
            .expect_err("超预算必须被拒");
        assert!(error.starts_with("CLOUD_GRAB_BUDGET_EXHAUSTED"), "{error}");
    }

    /// `search_source` 返回行 id 与页码（模型靠它才能引用原文）。
    #[test]
    fn search_source_returns_line_ids_and_pages() {
        let mut budget = GrabBudget::new();
        let result = search_source(&source(), &json!({"query": "questions 14-20"}), &mut budget)
            .expect("搜索必须成功");
        let hits = result["hits"].as_array().expect("hits");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["lineId"], json!("p2:l1"));
        assert_eq!(hits[0]["pageIndex"], json!(2));
        assert_eq!(hits[0]["text"], json!("Questions 14-20"));
    }

    /// 搜不到时如实返回空命中，不是错误。
    #[test]
    fn search_source_reports_no_hits_instead_of_failing() {
        let mut budget = GrabBudget::new();
        let result = search_source(&source(), &json!({"query": "nothing like this"}), &mut budget)
            .expect("搜不到不是错误");
        assert_eq!(result["hits"], json!([]));
        assert!(result["note"].as_str().unwrap().contains("No line"));
    }

    /// 只给引文时按引文定位到那一页。
    #[test]
    fn read_source_with_only_a_quote_locates_the_page() {
        let mut budget = GrabBudget::new();
        let result = read_source(&source(), &json!({"quote": "14 First question"}), &mut budget)
            .expect("引文能定位");
        assert_eq!(result["pages"][0]["pageIndex"], json!(2));
    }

    /// `read_passage` 必须能按题号定位，而不是只能返回开头一段。
    #[test]
    fn read_passage_locates_by_question_number_and_by_label() {
        let mut budget = GrabBudget::new();
        let by_number = read_passage(&source(), &json!({"questionNumbers": [14]}), &mut budget)
            .expect("按题号取段落");
        let paragraphs = by_number["paragraphs"].as_array().unwrap();
        assert_eq!(paragraphs.len(), 2, "题面与答案页上的 14 都要能找到：{paragraphs:#?}");
        assert!(paragraphs
            .iter()
            .any(|entry| entry["pageIndex"] == json!(3)));

        let by_label = read_passage(&source(), &json!({"paragraphLabels": ["A"]}), &mut budget)
            .expect("按标号取段落");
        assert!(!by_label["paragraphs"].as_array().unwrap().is_empty());
    }

    /// 两个选择器都不给时必须被拒。
    #[test]
    fn read_passage_without_a_selector_is_rejected() {
        let mut budget = GrabBudget::new();
        let error = read_passage(&source(), &json!({}), &mut budget).expect_err("必须被拒");
        assert!(error.starts_with("CLOUD_GRAB_PASSAGE_SELECTOR_REQUIRED"), "{error}");
    }

    /// `report_insufficient_context` 的 pages 需求要能被真的满足，且超预算时如实报不满足。
    #[test]
    fn satisfy_needs_fetches_pages_and_reports_what_it_could_not_get() {
        let root = std::env::temp_dir().join(format!("grab-needs-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&root).unwrap();
        let mut budget = GrabBudget::new();
        let (satisfied, unsatisfied, deferred) = satisfy_needs(
            &root,
            "job-1",
            &source(),
            &[
                json!({"kind": "pages", "from": 3, "to": 3}),
                json!({"kind": "pages", "from": 99, "to": 99}),
            ],
            &mut budget,
        );
        assert_eq!(satisfied.len(), 1, "第 3 页要真的取到");
        assert_eq!(satisfied[0]["result"]["pages"][0]["pageIndex"], json!(3));
        assert_eq!(unsatisfied.len(), 1, "越界页要如实报不满足");
        assert!(unsatisfied[0].starts_with("CLOUD_GRAB_PAGE_OUT_OF_RANGE"));
        assert!(deferred.is_empty(), "原文需求不该被推迟");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `candidate` / `draft` 两类需求本模块读不到：必须**推迟**给编排层，而不是报成
    /// 「没取到」——报成没取到会让模型以为后端也拿不到候选切片。
    #[test]
    fn satisfy_needs_defers_candidate_and_draft_to_the_orchestrator() {
        let root = std::env::temp_dir().join(format!("grab-defer-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&root).unwrap();
        let mut budget = GrabBudget::new();
        let (satisfied, unsatisfied, deferred) = satisfy_needs(
            &root,
            "job-1",
            &source(),
            &[
                json!({"kind": "candidate", "questionNumbers": [14]}),
                json!({"kind": "draft", "questionNumbers": [15]}),
            ],
            &mut budget,
        );
        assert!(satisfied.is_empty());
        assert!(unsatisfied.is_empty(), "推迟不是失败：{unsatisfied:?}");
        assert_eq!(deferred.len(), 2);
        assert_eq!(deferred[0]["kind"], json!("candidate"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 裁剪失败（这里用不存在的图）必须退回整页图并如实说明，而不是让整包失败。
    #[test]
    fn materialize_regions_falls_back_to_the_full_page_image() {
        let root = std::env::temp_dir().join(format!("grab-region-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&root).unwrap();
        let regions = vec![json!({
            "pageIndex": 2,
            "bbox": {"x": 10.0, "y": 10.0, "width": 100.0, "height": 20.0},
            "taskId": "tg-1",
        })];
        let out = materialize_regions(&root, "job-1", "pkt-1", &source(), &regions);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0]["image"]["path"],
            json!("/tmp/does-not-exist-2.png"),
            "裁不出来时退回整页图"
        );
        assert!(out[0]["note"].as_str().unwrap().contains("READ_FAILED"));
        assert_eq!(out[0]["taskIds"], json!(["tg-1"]));
        let _ = std::fs::remove_dir_all(&root);
    }
}
