//! M2（原 P6-T01 前半 + 计划 §12.2）：`import_files` —— 后端接管批量导入调度。
//!
//! 每份文件独立处理：建 job → 文件落地（hash 在事务外）→ library 外壳 → 入队。
//! 单文件失败只影响该文件（rejected），其余照常建行并入队；批量建行在秒级完成，
//! 解析由调度器在后台按并发上限执行。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::job_store::{make_job, save_job};
use crate::library::repository::{
    open_library_connection, set_item_status, upsert_item_shell, UpsertItemInput,
};
use crate::{app_root, CommandResult, CreateJobInput};

const MAX_IMPORT_FILE_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) const DEFAULT_CLOUD_TOKEN_BUDGET: u64 = 300_000;
pub(crate) const MIN_CLOUD_TOKEN_BUDGET: u64 = 10_000;
pub(crate) const MAX_CLOUD_TOKEN_BUDGET: u64 = 20_000_000;
pub(crate) const CLOUD_TOKEN_BUDGET_FILE: &str = "llm-token-budget.json";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportFileInput {
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub title_hint: Option<String>,
    #[serde(default)]
    pub answer_files: Vec<String>,
    /// 阅读整套卷子：按 `READING PASSAGE n` 标题把这份 PDF 拆成多篇，各建一条条目。
    /// 用户在导入抽屉里确认；确认后无法拆分时明确返回失败，允许取消拆分再导入。
    #[serde(default)]
    pub split_passages: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportFilesInput {
    pub files: Vec<ImportFileInput>,
    #[serde(default)]
    pub cloud_enabled: Option<bool>,
    #[serde(default)]
    pub cloud_profile_id: Option<String>,
    /// `reading` (default) or `listening`; confirmed by the user in the import flow.
    #[serde(default)]
    pub modality: Option<String>,
    #[serde(default)]
    pub cloud_token_budget: Option<u64>,
    /// `direct` (default) or `legacy`: the recognition chain snapshot at import
    /// time (cloud-direct plan §2). Jobs keep the mode they were queued with.
    #[serde(default)]
    pub pipeline_mode: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportCreatedItem {
    pub item_id: String,
    pub title: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportRejectedFile {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportFilesResult {
    pub created: Vec<ImportCreatedItem>,
    pub rejected: Vec<ImportRejectedFile>,
}

pub(crate) async fn import_files_core(
    app: &AppHandle,
    input: ImportFilesInput,
) -> CommandResult<ImportFilesResult> {
    let root = app_root(app)?;
    tauri::async_runtime::spawn_blocking(move || import_files_at_root(&root, input))
        .await
        .map_err(|error| error.to_string())?
}

pub(crate) fn import_files_at_root(
    root: &std::path::Path,
    input: ImportFilesInput,
) -> CommandResult<ImportFilesResult> {
    let cloud_enabled = input.cloud_enabled.unwrap_or(false);
    let modality = normalize_import_modality(input.modality.as_deref())?;
    let cloud_token_budget = normalize_cloud_token_budget(input.cloud_token_budget)?;
    // 导入时把流水线模式快照进 job progress：切换设置不改变已在处理中的作业链路。
    let pipeline_mode = normalize_import_pipeline_mode(input.pipeline_mode.as_deref())?;
    let mut created = Vec::new();
    let mut rejected = Vec::new();

    for file in input.files {
        let title = import_title(&file);
        if file.size_bytes > MAX_IMPORT_FILE_BYTES {
            rejected.push(ImportRejectedFile {
                name: file.name.clone(),
                reason: format!("超过导入上限 {}", format_bytes(MAX_IMPORT_FILE_BYTES)),
            });
            continue;
        }
        let options = SourceImportOptions {
            cloud_enabled,
            cloud_profile_id: input.cloud_profile_id.as_deref(),
            cloud_token_budget,
            modality,
            pipeline_mode,
        };
        if file.split_passages && modality == "reading" {
            let parts = match stage_passage_split(root, &file, &title) {
                Ok(parts) => parts,
                Err(reason) => {
                    rejected.push(ImportRejectedFile { name: file.name.clone(), reason });
                    continue;
                }
            };
            for part in &parts {
                let source = SourceToImport {
                    path: &part.path,
                    name: &part.name,
                    answer_files: &file.answer_files,
                    part: Some(&part.part),
                    split_answers: part.answer_path.as_deref(),
                };
                match import_one_source(root, &source, &part.title, &options) {
                    Ok(item) => created.push(item),
                    Err(reason) => rejected.push(ImportRejectedFile {
                        name: part.name.clone(),
                        reason,
                    }),
                }
            }
            remove_split_parts(&parts);
            continue;
        }
        let source = SourceToImport {
            path: &file.path,
            name: &file.name,
            answer_files: &file.answer_files,
            part: None,
            split_answers: None,
        };
        match import_one_source(root, &source, &title, &options) {
            Ok(item) => created.push(item),
            Err(reason) => rejected.push(ImportRejectedFile {
                name: file.name.clone(),
                reason,
            }),
        }
    }

    Ok(ImportFilesResult { created, rejected })
}

fn import_title(file: &ImportFileInput) -> String {
    let title = file
        .title_hint
        .clone()
        .filter(|hint| !hint.trim().is_empty())
        .unwrap_or_else(|| file.name.trim_end_matches(['.', ' ']).to_string());
    let stem = file
        .name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(&file.name);
    if title.trim().is_empty() {
        stem.to_string()
    } else {
        title
    }
}

struct SourceImportOptions<'a> {
    cloud_enabled: bool,
    cloud_profile_id: Option<&'a str>,
    cloud_token_budget: u64,
    modality: &'static str,
    pipeline_mode: &'static str,
}

struct SourceToImport<'a> {
    path: &'a str,
    name: &'a str,
    answer_files: &'a [String],
    /// 拆分出来的篇章标签（P1/P2/P3），来自原文标题行；整份导入为 `None`。
    part: Option<&'a str>,
    /// 拆分时为本篇生成的答案文本（AnswerKey）。
    split_answers: Option<&'a str>,
}

/// 一篇拆出来的子 PDF（临时文件，入库后删除）。
struct SplitPart {
    path: String,
    name: String,
    title: String,
    part: String,
    /// 本篇分出来的答案文本（没能分出时为 `None`，答案页已附在子 PDF 里）。
    answer_path: Option<String>,
}

/// 已确认拆分的文件必须拆成多条；失败明确返回，不得静默改为整份导入。
fn stage_passage_split(
    root: &std::path::Path,
    file: &ImportFileInput,
    title: &str,
) -> CommandResult<Vec<SplitPart>> {
    let source = std::path::Path::new(&file.path);
    let plan = super::pdf_split::plan_pdf_file(source);
    if !plan.is_multi() {
        return Err("拆分未完成：重新检查时未找到多篇阅读，未建立整份条目。请重新选择文件检查，或取消拆分后整份导入。".into());
    }
    let stem = file
        .name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(&file.name)
        .to_string();
    let dir = root.join("tmp").join("import-split");
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("[import] split: create temp dir failed: {error}");
        return Err("拆分未完成：无法建立临时目录，请检查应用数据目录的写入权限或磁盘空间。".into());
    }
    // 没有标题提示时整份导入的标题带扩展名；拆出来的篇名不重复带「.pdf」。
    let base_title = match title.len().checked_sub(4).and_then(|cut| title.get(cut..).map(|ext| (cut, ext))) {
        Some((cut, ext)) if ext.eq_ignore_ascii_case(".pdf") => &title[..cut],
        _ => title,
    };
    let mut parts = Vec::new();
    for segment in &plan.segments {
        let suffix = segment.label.replace(" · ", " ");
        // 只去掉路径/保留字符，保留中文等原文件名（originalName 要给用户看）。
        let name: String = format!("{stem} - {suffix}.pdf")
            .chars()
            .map(|ch| if ch.is_control() || "<>:\"/\\|?*".contains(ch) { '-' } else { ch })
            .collect();
        // 每篇一个独立临时目录：staging 以文件名作为 originalName，不能带随机前缀。
        let part_dir = dir.join(uuid::Uuid::new_v4().simple().to_string());
        let path = part_dir.join(&name);
        // 本篇自己的答案（从总答案页按题号分出来的），作为该条目的 AnswerKey 源文件。
        let answer_path = segment.answer_text().map(|text| {
            let answer_name = name.trim_end_matches(".pdf").to_string() + " 答案.txt";
            (part_dir.join(answer_name), text)
        });
        let written = std::fs::create_dir_all(&part_dir)
            .map_err(|error| error.to_string())
            .and_then(|_| super::pdf_split::write_segment_pdf(source, &plan, segment, &path))
            .and_then(|_| match &answer_path {
                Some((answer, text)) => std::fs::write(answer, text).map_err(|error| format!("split_write_answers:{error}")),
                None => Ok(()),
            });
        if let Err(error) = written {
            eprintln!("[import] split: {} segment {} failed: {error}", file.name, segment.label);
            remove_split_parts(&parts);
            let _ = std::fs::remove_dir_all(&part_dir);
            return Err("拆分未完成：无法生成各篇 PDF 或答案文件，未建立整份条目。请检查磁盘空间、目录权限及 PDF 是否损坏，然后重试；也可取消拆分后整份导入。".into());
        }
        parts.push(SplitPart {
            path: path.to_string_lossy().to_string(),
            name,
            title: format!("{base_title} · {}", segment.label),
            part: format!("P{}", segment.passage),
            answer_path: answer_path.map(|(answer, _)| answer.to_string_lossy().to_string()),
        });
    }
    Ok(parts)
}

fn remove_split_parts(parts: &[SplitPart]) {
    for part in parts {
        if let Some(dir) = std::path::Path::new(&part.path).parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// 一份源文件 → 一条条目：建 job → 文件落地 → library 外壳 → 入队。失败时补偿清理。
fn import_one_source(
    root: &std::path::Path,
    file: &SourceToImport<'_>,
    title: &str,
    options: &SourceImportOptions<'_>,
) -> Result<ImportCreatedItem, String> {
        // 1. 建 job（元数据落 job.json，与既有链共用）。
        let job = make_job(CreateJobInput {
            title: Some(title.to_string()),
            category: None,
            frequency: None,
            tags: None,
            llm_profile_id: None,
        });
        let job_id = job.job_id.clone();
        let staged = (|| -> CommandResult<()> {
            save_job(root, &job)?;
            let budget_path = crate::util::job_dir(root, &job_id).join(CLOUD_TOKEN_BUDGET_FILE);
            std::fs::write(
                budget_path,
                json!({"tokenBudget": options.cloud_token_budget}).to_string(),
            )
            .map_err(|error| format!("import_cloud_token_budget_save:{error}"))?;
            // 2. 文件落地：staging + hash 全部同步完成（数据库事务外，计划 §12.2）。
            crate::job_commands::stage_source_file(root, &job_id, file.path, "MainQuestion")?;
            for answer in file.answer_files {
                let extension = std::path::Path::new(answer).extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
                if !["pdf","png","jpg","jpeg","txt","md"].contains(&extension.as_str()) { return Err("答案文件支持 PDF、PNG、JPEG、TXT、MD".into()); }
                let size = std::fs::metadata(answer).map_err(|e| e.to_string())?.len();
                if size > MAX_IMPORT_FILE_BYTES { return Err("答案文件超过导入上限".into()); }
                crate::job_commands::stage_source_file(root, &job_id, answer, "AnswerKey")?;
            }
            if let Some(answers) = file.split_answers {
                crate::job_commands::stage_source_file(root, &job_id, answers, "AnswerKey")?;
            }
            Ok(())
        })();
        if let Err(error) = staged {
            // G1 边界：save_job/staging 失败同样会留下 job 壳（目录 + job.json），
            // 与 queue 失败同口径补偿清理，不留磁盘孤儿。
            compensate_failed_import(root, &job_id);
            return Err(error);
        }

        // 3. 数据库：library 外壳 + 处理任务入队（两个独立短写，文件操作已在外完成）。
        let queue_result: CommandResult<()> = queue_import(
            root,
            &job_id,
            title,
            file.name,
            options.cloud_enabled,
            options.cloud_profile_id,
            options.cloud_token_budget,
            options.modality,
            options.pipeline_mode,
            file.part,
        );
        if let Err(error) = queue_result {
            // G1/A4-F01：queue 失败必须补偿——磁盘 job 目录（含 staged 文件）
            // 与可能残留的 DB 行一并清除，不留「磁盘有 job、DB 无可见行」的孤儿；
            // 以 rejected 明确告知用户（原始文件未被移动或修改）。
            compensate_failed_import(root, &job_id);
            return Err(error);
        }
    Ok(ImportCreatedItem {
        item_id: job_id,
        title: title.to_string(),
    })
}

/// G1/A4-F01 补偿删除：queue_import 失败后清空本次导入的痕迹。
/// queue_import 是单事务，正常失败时 DB 无行；commit 歧义时可能有行残留，
/// 因此 DB 清理尽力而为。job 目录（job.json + uploads staged 文件）一并删除。
fn compensate_failed_import(root: &std::path::Path, job_id: &str) {
    let dir = crate::util::job_dir(root, job_id);
    if dir.exists() {
        if let Err(error) = std::fs::remove_dir_all(&dir) {
            eprintln!("[import] compensate: remove job dir {job_id} failed: {error}");
        }
    }
    if let Ok(mut conn) = open_library_connection(root) {
        // 复核 B/F：两条 DELETE 包同一事务——第一条成功、第二条失败会留下
        // 无 job 的 item 壳，状态永远"排队中"且无兜底清理。
        let tx = conn.transaction();
        let tx = match tx {
            Ok(tx) => tx,
            Err(error) => {
                eprintln!("[import] compensate: db transaction failed for {job_id}: {error}");
                return;
            }
        };
        if let Err(error) = tx.execute("DELETE FROM processing_jobs_v2 WHERE id = ?1", [job_id]) {
            eprintln!("[import] compensate: delete queue row {job_id} failed: {error}");
        }
        if let Err(error) = tx.execute("DELETE FROM library_items_v2 WHERE id = ?1", [job_id]) {
            eprintln!("[import] compensate: delete item shell {job_id} failed: {error}");
        }
        if let Err(error) = tx.commit() {
            eprintln!("[import] compensate: commit {job_id} failed: {error}");
        }
    }
}

/// 数据库收尾：library 外壳 + 处理任务入队（独立短写）。
fn queue_import(
    root: &std::path::Path,
    job_id: &str,
    title: &str,
    file_name: &str,
    cloud_enabled: bool,
    cloud_profile_id: Option<&str>,
    cloud_token_budget: u64,
    modality: &str,
    pipeline_mode: &str,
    part: Option<&str>,
) -> CommandResult<()> {
    let conn = open_library_connection(root)?;
    let transaction =
        rusqlite::Transaction::new_unchecked(&conn, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
    upsert_item_shell(
        &transaction,
        &UpsertItemInput {
            id: job_id,
            modality,
            title,
            status: "processing",
            source_asset_id: None,
        },
    )?;
    set_item_status(&transaction, job_id, "processing")?;
    if let Some(part) = part {
        crate::library::repository::set_item_part(&transaction, job_id, Some(part), Some("content"))?;
    }
    super::queue::enqueue(
        &transaction,
        job_id,
        job_id,
        job_id,
        &json!({
            "cloudEnabled": cloud_enabled,
            "fileName": file_name,
            "cloudProfileId": cloud_profile_id,
            "cloudTokenBudget": cloud_token_budget,
            "modality": modality,
            "pipelineMode": pipeline_mode
        }),
    )?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(())
}

/// Import accepts exactly the paper modalities this pipeline can host; anything else is a
/// caller bug and fails the whole request before any job is created.
pub(crate) fn normalize_import_modality(modality: Option<&str>) -> CommandResult<&'static str> {
    match modality.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("reading") => Ok("reading"),
        Some("listening") => Ok("listening"),
        Some(other) => Err(format!("import_modality_unsupported:{other}")),
    }
}

/// 导入入参的流水线模式归一：无法识别时**不猜**，直接拒绝这次导入
/// （模式决定整条识别链，猜错比报错危险）。
pub(crate) fn normalize_import_pipeline_mode(mode: Option<&str>) -> CommandResult<&'static str> {
    match mode.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(crate::pipeline_settings::PIPELINE_MODE_DIRECT),
        Some(crate::pipeline_settings::PIPELINE_MODE_DIRECT) => {
            Ok(crate::pipeline_settings::PIPELINE_MODE_DIRECT)
        }
        Some(crate::pipeline_settings::PIPELINE_MODE_LEGACY) => {
            Ok(crate::pipeline_settings::PIPELINE_MODE_LEGACY)
        }
        Some(other) => Err(format!("import_pipeline_mode_unsupported:{other}")),
    }
}

pub(crate) fn normalize_cloud_token_budget(budget: Option<u64>) -> CommandResult<u64> {
    let budget = budget.unwrap_or(DEFAULT_CLOUD_TOKEN_BUDGET);
    if !(MIN_CLOUD_TOKEN_BUDGET..=MAX_CLOUD_TOKEN_BUDGET).contains(&budget) {
        return Err(format!(
            "import_cloud_token_budget_out_of_range:{MIN_CLOUD_TOKEN_BUDGET}:{MAX_CLOUD_TOKEN_BUDGET}"
        ));
    }
    Ok(budget)
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{} MB", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

/// import_files 的返回转 Value 供 Tauri 命令层使用。
pub(crate) fn import_files_result_to_value(result: ImportFilesResult) -> Value {
    serde_json::to_value(result).unwrap_or(json!({"created": [], "rejected": []}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("import-comp-{}", Uuid::new_v4().simple()))
    }

    #[test]
    fn cloud_token_budget_uses_the_default_and_keeps_explicit_values() {
        assert_eq!(
            normalize_cloud_token_budget(None).unwrap(),
            DEFAULT_CLOUD_TOKEN_BUDGET
        );
        assert_eq!(normalize_cloud_token_budget(Some(75_000)).unwrap(), 75_000);
    }

    /// G1/A4-F01：queue 失败（DB 打不开）时补偿删除 job 目录与 DB 残留，
    /// 不留「磁盘有 job、DB 无可见行」的孤儿；用户拿到明确的 rejected 结果。
    #[test]
    fn queue_failure_compensates_and_rejects() {
        let root = temp_root();
        fs::create_dir_all(&root).unwrap();
        // 失败注入：把 authoring_hub.db 占位成目录 → Connection::open 必然失败。
        fs::create_dir_all(root.join("authoring_hub.db")).unwrap();
        let source = root.join("sample.pdf");
        fs::write(&source, b"%PDF-1.4 fake").unwrap();

        let input = ImportFilesInput {
            files: vec![ImportFileInput {
                path: source.to_string_lossy().to_string(),
                name: "sample.pdf".to_string(),
                size_bytes: 13,
                title_hint: None,
                answer_files: vec![],
                split_passages: false,
            }],
            cloud_enabled: Some(false),
            cloud_profile_id: None,
            modality: None,
            cloud_token_budget: None,
            pipeline_mode: None,
        };
        let result = import_files_at_root(&root, input).unwrap();
        assert!(
            result.created.is_empty(),
            "queue 失败的文件不得计入 created"
        );
        assert_eq!(result.rejected.len(), 1, "失败必须以 rejected 明确告知");
        let jobs_dir = root.join("jobs");
        let leftover = fs::read_dir(&jobs_dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(leftover, 0, "queue 失败后不得留下 job 目录孤儿");
        let _ = fs::remove_dir_all(&root);
    }

    /// 整套卷子测试 PDF：每篇「正文页（标题 + 说明行 + 正文）→ 题目页」，末尾一页总答案
    /// （先听力区再阅读区）。`statements` 给出每篇的题号起止。
    fn whole_test_pdf(passages: &[(&str, u32, u32)]) -> Vec<u8> {
        let mut pages: Vec<Vec<String>> = Vec::new();
        for (index, (topic, first, last)) in passages.iter().enumerate() {
            let n = index + 1;
            let mut prose = vec![
                format!("READING PASSAGE {n}"),
                format!("You should spend about 20 minutes on Questions {first}-{last}, which are based on Reading Passage {n} below."),
                format!("The story of {topic}"),
            ];
            prose.extend(super::super::pdf_split::prose_lines(topic));
            pages.push(prose);
            let mut questions = vec![
                format!("Questions {first}-{last}"),
                format!("Do the following statements agree with the information given in Reading Passage {n}?"),
                "Write TRUE if the statement agrees with the information".to_string(),
                "FALSE if the statement contradicts the information".to_string(),
                "NOT GIVEN if there is no information on this".to_string(),
            ];
            for q in *first..=*last {
                questions.push(format!("{q} The writer describes {topic} fact number {q} in the passage."));
            }
            pages.push(questions);
        }
        let mut key = vec!["Listening and Reading Answer Keys".to_string(), "LISTENING".to_string()];
        for q in 1..=10 {
            key.push(format!("{q} listening answer"));
        }
        key.push("READING".to_string());
        for (index, (_, first, last)) in passages.iter().enumerate() {
            key.push(format!("Reading Passage {}, Questions {first}-{last}", index + 1));
            for q in *first..=*last {
                key.push(format!("{q} {}", ["TRUE", "FALSE", "NOT GIVEN"][(q % 3) as usize]));
            }
        }
        pages.push(key);
        let owned: Vec<Vec<&str>> = pages.iter().map(|p| p.iter().map(String::as_str).collect()).collect();
        let refs: Vec<&[&str]> = owned.iter().map(Vec::as_slice).collect();
        super::super::pdf_split::minimal_text_pdf(&refs)
    }

    fn split_import(root: &std::path::Path, name: &str, bytes: &[u8], title_hint: Option<&str>) -> ImportFilesResult {
        let source = root.join(name);
        fs::write(&source, bytes).unwrap();
        import_files_at_root(
            root,
            ImportFilesInput {
                files: vec![ImportFileInput {
                    path: source.to_string_lossy().to_string(),
                    name: name.to_string(),
                    size_bytes: bytes.len() as u64,
                    title_hint: title_hint.map(str::to_string),
                    answer_files: vec![],
                    split_passages: true,
                }],
                cloud_enabled: Some(false),
                cloud_profile_id: None,
                modality: None,
                cloud_token_budget: None,
                pipeline_mode: None,
            },
        )
        .unwrap()
    }

    /// 一份 PDF 里的整套阅读卷子 → 每篇一条条目；子 PDF 只含本篇页，
    /// 答案按题号分开、作为各自的 AnswerKey 文本，听力区的答案不进来。
    #[test]
    fn whole_test_pdf_splits_into_one_item_per_passage() {
        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        let bytes = whole_test_pdf(&[("glass", 1, 13), ("horses", 14, 26), ("sleep", 27, 40)]);
        let result = split_import(&root, "剑桥 Test 1.pdf", &bytes, None);
        let reasons: Vec<_> = result.rejected.iter().map(|r| r.reason.clone()).collect();
        assert!(reasons.is_empty(), "rejected: {reasons:?}");
        let titles: Vec<_> = result.created.iter().map(|item| item.title.clone()).collect();
        assert_eq!(titles, ["剑桥 Test 1 · P1", "剑桥 Test 1 · P2", "剑桥 Test 1 · P3"]);
        let ranges = [(1u32, 13u32), (14, 26), (27, 40)];
        for (item, (first, last)) in result.created.iter().zip(ranges) {
            let job = crate::job_store::load_job(&root, &item.item_id).unwrap();
            let uploads = crate::util::job_dir(&root, &item.item_id).join("uploads");
            let main = job.source_files.iter().find(|s| s.role == "MainQuestion").unwrap();
            assert!(main.original_name.starts_with("剑桥 Test 1 - P"));
            let text = pdf_extract::extract_text_by_pages(&uploads.join(&main.stored_name)).unwrap();
            assert_eq!(text.len(), 2, "{}：子 PDF 只含本篇两页，不再附整页答案", item.title);
            assert!(!text.iter().any(|page| page.contains("Answer Keys")));
            let key = job.source_files.iter().find(|s| s.role == "AnswerKey").expect("本篇答案");
            let answers = fs::read_to_string(uploads.join(&key.stored_name)).unwrap();
            let numbers: Vec<u32> = answers
                .lines()
                .filter_map(|line| line.split_whitespace().next()?.parse().ok())
                .collect();
            assert_eq!(numbers, (first..=last).collect::<Vec<_>>(), "{}", item.title);
            assert!(!answers.contains("listening"), "听力答案不能进阅读条目");
        }
        let leftovers = fs::read_dir(root.join("tmp").join("import-split"))
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(leftovers, 0, "临时子 PDF / 答案文本已清理");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn confirmed_split_staging_failure_does_not_import_whole() {
        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        fs::create_dir_all(root.join("tmp")).unwrap();
        fs::write(root.join("tmp/import-split"), b"blocked directory").unwrap();
        let bytes = whole_test_pdf(&[("glass", 1, 13), ("horses", 14, 26)]);
        let result = split_import(&root, "full.pdf", &bytes, None);
        assert!(result.created.is_empty(), "must not silently import whole");
        assert_eq!(result.rejected.len(), 1);
        assert!(result.rejected[0].reason.contains("拆分未完成"));
        assert!(item_rows(&root).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    /// 拆出来的第二篇走真实本地识别链：识别成 P2、题号从 14 开始、答案来自分出来的
    /// 本篇答案，第一篇的题和答案都不混进来。
    #[test]
    fn split_passage_runs_through_local_recognition_as_its_own_part() {
        if crate::pdf_geometry::pdfium_library_path().is_none() {
            return;
        }
        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        let bytes = whole_test_pdf(&[("glass", 1, 3), ("horses", 14, 16)]);
        let result = split_import(&root, "full.pdf", &bytes, Some("Full"));
        assert_eq!(result.created.len(), 2, "{:?}", result.rejected.iter().map(|r| &r.reason).collect::<Vec<_>>());
        let second = &result.created[1].item_id;
        crate::auto_pipeline::run_auto_pipeline_core(
            &root,
            second,
            Some(crate::AutoPipelineInput {
                execution_mode: Some("localOnly".to_string()),
                target: Some("editableDraft".to_string()),
                ..Default::default()
            }),
        )
        .expect("local pipeline on a split passage");
        let session = crate::authoring_v2_commands::get_authoring_v2_core(&root, second).unwrap();
        let authoring = session.get("authoring").cloned().unwrap();
        let text = authoring.to_string();
        assert!(text.contains("horses"), "{text}");
        assert!(!text.contains("glass fact number"), "第一篇的题不能混进第二篇");
        let slots = authoring.get("answerSlots").and_then(Value::as_object).cloned().unwrap_or_default();
        let mut numbers: Vec<u64> = slots.values().filter_map(|slot| slot.get("questionNumber").and_then(Value::as_u64)).collect();
        numbers.sort_unstable();
        assert_eq!(numbers, vec![14, 15, 16], "{text}");
        // 答案来自本篇分出来的答案文本：14 % 3 = 2 → NOT GIVEN，15 → TRUE，16 → FALSE。
        let key = authoring.get("answerKey").cloned().unwrap_or(Value::Null).to_string();
        for expected in ["FALSE", "NOT GIVEN", "TRUE"] {
            assert!(key.to_ascii_uppercase().contains(expected) || key.contains(&expected.to_ascii_lowercase()), "answerKey={key}");
        }
        assert!(!key.contains("listening"), "answerKey={key}");
        assert_eq!(authoring.pointer("/exam/category").and_then(Value::as_str), Some("P2"));
        let conn = open_library_connection(&root).unwrap();
        let row = crate::library::repository::get_item(&conn, second).unwrap().unwrap();
        assert_eq!(row.part_label.as_deref(), Some("P2"));
        let _ = fs::remove_dir_all(&root);
    }

    fn import_one(root: &std::path::Path, modality: Option<&str>) -> String {
        crate::util::ensure_app_dirs(root).unwrap();
        let source = root.join("paper.pdf");
        fs::write(&source, b"%PDF-1.4 fake").unwrap();
        let input = ImportFilesInput {
            files: vec![ImportFileInput {
                path: source.to_string_lossy().to_string(),
                name: "paper.pdf".to_string(),
                size_bytes: 13,
                title_hint: None,
                answer_files: vec![],
                split_passages: false,
            }],
            cloud_enabled: Some(false),
            cloud_profile_id: None,
            modality: modality.map(str::to_string),
            cloud_token_budget: None,
            pipeline_mode: None,
        };
        let result = import_files_at_root(root, input).unwrap();
        assert_eq!(
            result.created.len(),
            1,
            "rejected: {:?}",
            result
                .rejected
                .iter()
                .map(|r| &r.reason)
                .collect::<Vec<_>>()
        );
        result.created[0].item_id.clone()
    }

    /// Simulates the pipeline's local recognition output (always built as a
    /// reading-shaped draft today) and runs the real first-seed entry point.
    fn seed_pipeline_draft(root: &std::path::Path, item_id: &str) -> Value {
        let authoring = json!({
            "schemaVersion": "IeltsAuthoringIRV2",
            "modality": "reading",
            "exam": {"title": "Paper"},
            "taskGroups": []
        });
        fs::write(
            crate::util::job_dir(root, item_id)
                .join(crate::authoring_v2_commands::AUTHORING_V2_SHADOW_FILE),
            serde_json::to_vec(&authoring).unwrap(),
        )
        .unwrap();
        assert!(crate::library::migration::ensure_initial_canonical(root, item_id).unwrap());
        let conn = open_library_connection(root).unwrap();
        crate::library::repository::get_canonical_ds(&conn, item_id)
            .unwrap()
            .unwrap()
            .0
    }

    fn item_rows(root: &std::path::Path) -> Vec<(String, String)> {
        let conn = open_library_connection(root).unwrap();
        let mut stmt = conn
            .prepare("SELECT id, modality FROM library_items_v2")
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn listening_import_creates_one_listening_item_and_listening_canonical() {
        let root = temp_root();
        let item_id = import_one(&root, Some("listening"));
        let rows = item_rows(&root);
        assert_eq!(rows, vec![(item_id.clone(), "listening".to_string())]);
        let conn = open_library_connection(&root).unwrap();
        let payload: String = conn
            .query_row(
                "SELECT progress_json FROM processing_jobs_v2 WHERE id = ?1",
                [&item_id],
                |row| row.get(0),
            )
            .unwrap();
        drop(conn);
        assert_eq!(
            serde_json::from_str::<Value>(&payload).unwrap()["modality"],
            "listening"
        );
        let canonical = seed_pipeline_draft(&root, &item_id);
        assert_eq!(canonical["modality"], "listening");
        assert_eq!(
            item_rows(&root).len(),
            1,
            "seeding must not create a second item"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn reading_import_stays_reading_by_default() {
        let root = temp_root();
        let item_id = import_one(&root, None);
        assert_eq!(
            item_rows(&root),
            vec![(item_id.clone(), "reading".to_string())]
        );
        let canonical = seed_pipeline_draft(&root, &item_id);
        assert_eq!(canonical["modality"], "reading");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unknown_import_modality_is_rejected() {
        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        let source = root.join("paper.pdf");
        fs::write(&source, b"%PDF-1.4 fake").unwrap();
        let input = ImportFilesInput {
            files: vec![ImportFileInput {
                path: source.to_string_lossy().to_string(),
                name: "paper.pdf".to_string(),
                size_bytes: 13,
                title_hint: None,
                answer_files: vec![],
                split_passages: false,
            }],
            cloud_enabled: Some(false),
            cloud_profile_id: None,
            modality: Some("writing".to_string()),
            cloud_token_budget: None,
            pipeline_mode: None,
        };
        assert!(import_files_at_root(&root, input).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    /// G1 边界：staging 本身失败（源文件不存在）时，save_job 已写下的
    /// job 壳同样要被补偿清理——与 queue 失败同口径，不留磁盘孤儿。
    #[test]
    fn staging_failure_compensates_job_shell() {
        let root = temp_root();
        fs::create_dir_all(&root).unwrap();

        let input = ImportFilesInput {
            files: vec![ImportFileInput {
                path: root
                    .join("missing-source.pdf")
                    .to_string_lossy()
                    .to_string(),
                name: "missing-source.pdf".to_string(),
                size_bytes: 0,
                title_hint: None,
                answer_files: vec![],
                split_passages: false,
            }],
            cloud_enabled: Some(false),
            cloud_profile_id: None,
            modality: None,
            cloud_token_budget: None,
            pipeline_mode: None,
        };
        let result = import_files_at_root(&root, input).unwrap();
        assert!(result.created.is_empty());
        assert_eq!(
            result.rejected.len(),
            1,
            "staging 失败必须以 rejected 明确告知"
        );
        let jobs_dir = root.join("jobs");
        let leftover = fs::read_dir(&jobs_dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(leftover, 0, "staging 失败后不得留下 job 壳目录");
        let _ = fs::remove_dir_all(&root);
    }
}
