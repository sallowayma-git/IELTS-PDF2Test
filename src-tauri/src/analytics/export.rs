//! Excel 导出（rust_xlsxwriter）。工作表：成绩汇总 / 学生明细 / 题目分析 / 题型统计 / 导入异常。
//!
//! 只写用户在保存对话框里选的路径；不碰作答数据目录。

use std::path::Path;

use rusqlite::Connection;
use rust_xlsxwriter::{Format, Workbook};

use super::import::list_failed_imports_for_group;
use super::queries::{exam_view_from, load_attempts, load_attempts_with_mode, AttemptRec, ExamView, Scope};
use super::{display_answer, DISCLAIMER, HARD_CONCLUSION_MIN_N};
use crate::CommandResult;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub path: String,
    pub sheets: Vec<String>,
    pub n: usize,
}

/// 题型标识 → 中文（与前端 `src/utils/displayLabels.ts` 的 taskTypeLabels 保持一致）。
pub fn task_type_label(kind: &str) -> String {
    let label = match kind {
        "" | "unknown" => "题型未标注",
        "single_choice" => "单选题",
        "multiple_choice" => "多选题",
        "true_false_not_given" => "判断题（TRUE / FALSE / NOT GIVEN）",
        "yes_no_not_given" => "判断题（YES / NO / NOT GIVEN）",
        "matching_information" => "段落信息匹配",
        "matching_headings" => "段落标题匹配",
        "matching_features" => "特征匹配",
        "matching_sentence_endings" => "句子结尾匹配",
        "classification" => "分类匹配",
        "sentence_completion" => "句子填空",
        "summary_completion" => "摘要填空",
        "note_completion" => "笔记填空",
        "table_completion" => "表格填空",
        "form_completion" => "表单填空",
        "flowchart_completion" => "流程图填空",
        "diagram_label_completion" => "图表标注",
        "plan_map_label_completion" => "平面图标注",
        "short_answer" => "简答题",
        other => return other.to_string(),
    };
    label.to_string()
}

fn xerr(error: rust_xlsxwriter::XlsxError) -> String {
    format!("xlsx_write:{error}")
}

pub fn write_export(conn: &Connection, batch_id: &str, paper_id: &str, path: &Path) -> CommandResult<ExportSummary> {
    let attempts = load_attempts_with_mode(conn, &Scope::Group { batch_id, paper_id }, true)?;
    let view = exam_view_from(batch_id, paper_id, &attempts);
    let exceptions = list_failed_imports_for_group(conn, batch_id, paper_id)?;

    let mut workbook = Workbook::new();
    let bold = Format::new().set_bold();
    let wrap = Format::new().set_text_wrap();
    let sheets = vec![
        "成绩汇总".to_string(),
        "学生明细".to_string(),
        "题目分析".to_string(),
        "题型统计".to_string(),
        "导入异常".to_string(),
        "改判后成绩".to_string(),
        "写作成绩".to_string(),
        "听力".to_string(),
    ];

    write_scores(&mut workbook, &bold, &view)?;
    write_details(&mut workbook, &bold, &attempts)?;
    write_questions(&mut workbook, &bold, &view)?;
    write_kinds(&mut workbook, &bold, &view)?;
    write_exceptions(&mut workbook, &bold, &wrap, &exceptions)?;

    let effective=load_attempts(conn,&Scope::Group{batch_id,paper_id})?;
    let corrected:Vec<serde_json::Value>=effective.iter().map(|a|serde_json::json!({"姓名":a.candidate_name,"卡号":a.candidate_id,"考试记录":a.attempt_id,"改判后得分":a.raw_score(),"满分":a.raw_total(),"计入统计":a.is_valid(),"说明":a.invalid_reason})).collect();
    write_json_sheet(&mut workbook,"改判后成绩",&corrected,view.n)?;
    let writings=super::snapshot_v2::rows(conn,"SELECT a.candidate_name AS 姓名,a.candidate_id AS 卡号,w.task_type AS 写作任务,g.ta_tr AS 任务回应,g.cc AS 衔接,g.lr AS 词汇,g.gra AS 语法,g.overall_comment AS 总评,g.status AS 批改状态,g.version AS 版本,g.grader AS 批改人 FROM writings w JOIN attempts a USING(attempt_id) LEFT JOIN writing_grades g ON g.attempt_id=w.attempt_id AND g.writing_submission_id=w.writing_submission_id AND g.status='published' WHERE a.batch_id=? AND a.paper_id=?",&[&batch_id,&paper_id])?;
    let writing_n=writings.len();write_json_sheet(&mut workbook,"写作成绩",&writings,writing_n)?;
    let listening=super::snapshot_v2::rows(conn,"SELECT a.candidate_name AS 姓名,a.candidate_id AS 卡号,r.slot_id AS 计分单元,r.user_answer_json AS 学生答案,r.correct_answer_json AS 标准答案,r.score AS 原始得分,COALESCE(o.new_score,r.score) AS 改判后得分,r.max_score AS 满分,a.counted AS 计入统计 FROM listening_answers r JOIN attempts a USING(attempt_id) LEFT JOIN score_overrides o ON o.attempt_id=r.attempt_id AND o.modality='listening' AND o.passage_index=-1 AND o.response_slot_id=r.slot_id AND o.revoked_at IS NULL WHERE a.batch_id=? AND a.paper_id=? ORDER BY a.attempt_id,r.slot_id",&[&batch_id,&paper_id])?;
    let listening_n:usize=conn.query_row("SELECT COUNT(*) FROM listening_attempts l JOIN attempts a USING(attempt_id) WHERE a.batch_id=?1 AND a.paper_id=?2",rusqlite::params![batch_id,paper_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    write_json_sheet(&mut workbook,"听力",&listening,listening_n)?;
    workbook.save(path).map_err(xerr)?;
    Ok(ExportSummary {
        path: path.to_string_lossy().to_string(),
        sheets,
        n: view.n,
    })
}

fn write_scores(workbook: &mut Workbook, bold: &Format, view: &ExamView) -> CommandResult<()> {
    let sheet = workbook.add_worksheet();
    sheet.set_name("成绩汇总").map_err(xerr)?;
    sheet
        .write_string_with_format(0, 0, format!("{} · {} 成绩汇总（N={}）", view.batch_id, paper_text(&view.paper_id), view.n), bold)
        .map_err(xerr)?;
    sheet.write_string(1, 0, DISCLAIMER).map_err(xerr)?;

    let part_headers: Vec<String> = view.parts.iter().map(|p| format!("{}得分点", p.label)).collect();
    let mut headers: Vec<String> = ["姓名", "卡号", "原始分", "满分", "正确率"].iter().map(|s| s.to_string()).collect();
    headers.extend(part_headers);
    headers.push("交卷时间".into());
    for (col, header) in headers.iter().enumerate() {
        sheet.write_string_with_format(3, col as u16, header.as_str(), bold).map_err(xerr)?;
    }
    for (offset, student) in view.students.iter().enumerate() {
        let row = 4 + offset as u32;
        sheet.write_string(row, 0, &student.candidate_name).map_err(xerr)?;
        sheet.write_string(row, 1, &student.candidate_id).map_err(xerr)?;
        sheet.write_number(row, 2, student.raw_score).map_err(xerr)?;
        sheet.write_number(row, 3, student.raw_total).map_err(xerr)?;
        if let Some(accuracy) = student.accuracy {
            sheet.write_number(row, 4, accuracy).map_err(xerr)?;
        }
        for (index, value) in student.part_correct.iter().enumerate() {
            if let Some(value) = value {
                sheet.write_number(row, 5 + index as u16, *value).map_err(xerr)?;
            }
        }
        if let Some(submitted) = &student.submitted_at {
            sheet.write_string(row, (headers.len() - 1) as u16, submitted).map_err(xerr)?;
        }
    }
    let mut row = 4 + view.students.len() as u32 + 1;
    let summary = [
        ("均值", view.score.mean),
        ("中位数", view.score.median),
        ("最高", view.score.max),
        ("最低", view.score.min),
    ];
    for (label, value) in summary {
        sheet.write_string(row, 0, label).map_err(xerr)?;
        if let Some(value) = value {
            sheet.write_number(row, 2, value).map_err(xerr)?;
        }
        row += 1;
    }
    if !view.excluded.is_empty() {
        row += 1;
        sheet.write_string_with_format(row, 0, "不计入", bold).map_err(xerr)?;
        row += 1;
        for excluded in &view.excluded {
            sheet.write_string(row, 0, &excluded.candidate_name).map_err(xerr)?;
            sheet.write_string(row, 1, &excluded.attempt_id).map_err(xerr)?;
            sheet.write_string(row, 2, &excluded.reason).map_err(xerr)?;
            row += 1;
        }
    }
    sheet.set_column_width(0, 14.0).map_err(xerr)?;
    sheet.set_column_width(1, 16.0).map_err(xerr)?;
    sheet.set_column_width(5, 14.0).map_err(xerr)?;
    sheet.set_column_width((headers.len() - 1) as u16, 22.0).map_err(xerr)?;
    Ok(())
}

fn write_details(workbook: &mut Workbook, bold: &Format, attempts: &[AttemptRec]) -> CommandResult<()> {
    let sheet = workbook.add_worksheet();
    sheet.set_name("学生明细").map_err(xerr)?;
    sheet.write_string(0, 0, DISCLAIMER).map_err(xerr)?;
    let headers = ["姓名", "卡号", "篇", "题号", "题型", "学生答案", "标准答案", "判分(1对0错)", "得分点数", "停留(ms)", "改答次数", "访问次数"];
    for (col, header) in headers.iter().enumerate() {
        sheet.write_string_with_format(1, col as u16, *header, bold).map_err(xerr)?;
    }
    let mut row = 2u32;
    let mut ordered: Vec<&AttemptRec> = attempts.iter().filter(|a| a.is_valid()).collect();
    ordered.sort_by(|a, b| a.candidate_id.cmp(&b.candidate_id).then(a.attempt_id.cmp(&b.attempt_id)));
    for attempt in ordered {
        let mut answers: Vec<_> = attempt.answers.iter().collect();
        answers.sort_by(|a, b| (a.passage_index, a.question_id.len(), &a.question_id).cmp(&(b.passage_index, b.question_id.len(), &b.question_id)));
        for answer in answers {
            if answer.is_correct.is_none() {
                continue;
            }
            sheet.write_string(row, 0, &attempt.candidate_name).map_err(xerr)?;
            sheet.write_string(row, 1, &attempt.candidate_id).map_err(xerr)?;
            sheet.write_number(row, 2, (answer.passage_index + 1) as f64).map_err(xerr)?;
            sheet.write_string(row, 3, answer.display_label.as_deref().unwrap_or(&answer.question_id)).map_err(xerr)?;
            sheet.write_string(row, 4, task_type_label(answer.question_kind.as_deref().unwrap_or(""))).map_err(xerr)?;
            sheet.write_string(row, 5, display_answer(answer.user_answer_json.as_deref())).map_err(xerr)?;
            sheet.write_string(row, 6, display_answer(answer.correct_answer_json.as_deref())).map_err(xerr)?;
            sheet.write_number(row, 7, answer.is_correct.unwrap_or(0) as f64).map_err(xerr)?;
            sheet.write_number(row, 8, answer.weight).map_err(xerr)?;
            sheet.write_number(row, 9, answer.elapsed_ms as f64).map_err(xerr)?;
            sheet.write_number(row, 10, answer.change_count as f64).map_err(xerr)?;
            sheet.write_number(row, 11, answer.visit_count as f64).map_err(xerr)?;
            row += 1;
        }
    }
    sheet.set_column_width(4, 26.0).map_err(xerr)?;
    sheet.set_column_width(5, 22.0).map_err(xerr)?;
    sheet.set_column_width(6, 22.0).map_err(xerr)?;
    Ok(())
}

fn write_questions(workbook: &mut Workbook, bold: &Format, view: &ExamView) -> CommandResult<()> {
    let sheet = workbook.add_worksheet();
    sheet.set_name("题目分析").map_err(xerr)?;
    sheet.write_string(0, 0, format!("N 为参与统计的有效人数；N < {HARD_CONCLUSION_MIN_N} 时不下「难题」结论。")).map_err(xerr)?;
    sheet.write_string(1, 0, DISCLAIMER).map_err(xerr)?;
    let headers = ["篇", "题号", "题型", "N", "正确人数", "正确率", "结论"];
    for (col, header) in headers.iter().enumerate() {
        sheet.write_string_with_format(2, col as u16, *header, bold).map_err(xerr)?;
    }
    for (offset, question) in view.questions.iter().enumerate() {
        let row = 3 + offset as u32;
        sheet.write_string(row, 0, &question.passage_label).map_err(xerr)?;
        sheet.write_string(row, 1, &question.display_label).map_err(xerr)?;
        sheet.write_string(row, 2, task_type_label(&question.question_kind)).map_err(xerr)?;
        sheet.write_number(row, 3, question.n as f64).map_err(xerr)?;
        sheet.write_number(row, 4, question.correct_count as f64).map_err(xerr)?;
        if let Some(accuracy) = question.accuracy {
            sheet.write_number(row, 5, accuracy).map_err(xerr)?;
        }
        let conclusion = if question.hard_conclusion {
            if question.accuracy.unwrap_or(1.0) < 0.5 { "偏难" } else { "正常" }
        } else {
            "样本不足，不下结论"
        };
        sheet.write_string(row, 6, conclusion).map_err(xerr)?;
    }
    sheet.set_column_width(1, 12.0).map_err(xerr)?;
    sheet.set_column_width(2, 26.0).map_err(xerr)?;
    sheet.set_column_width(6, 22.0).map_err(xerr)?;
    Ok(())
}

fn write_kinds(workbook: &mut Workbook, bold: &Format, view: &ExamView) -> CommandResult<()> {
    let sheet = workbook.add_worksheet();
    sheet.set_name("题型统计").map_err(xerr)?;
    sheet.write_string(0, 0, "分母为得分点数（答案行 weight 之和）。").map_err(xerr)?;
    sheet.write_string(1, 0, DISCLAIMER).map_err(xerr)?;
    let headers = ["题型", "N", "得分点数", "得分点", "正确率"];
    for (col, header) in headers.iter().enumerate() {
        sheet.write_string_with_format(2, col as u16, *header, bold).map_err(xerr)?;
    }
    for (offset, kind) in view.kinds.iter().enumerate() {
        let row = 3 + offset as u32;
        sheet.write_string(row, 0, task_type_label(&kind.kind)).map_err(xerr)?;
        sheet.write_number(row, 1, kind.n as f64).map_err(xerr)?;
        sheet.write_number(row, 2, kind.points).map_err(xerr)?;
        sheet.write_number(row, 3, kind.correct_points).map_err(xerr)?;
        if let Some(accuracy) = kind.accuracy {
            sheet.write_number(row, 4, accuracy).map_err(xerr)?;
        }
    }
    sheet.set_column_width(0, 30.0).map_err(xerr)?;
    Ok(())
}

fn write_exceptions(
    workbook: &mut Workbook,
    bold: &Format,
    wrap: &Format,
    exceptions: &[super::import::ImportRecord],
) -> CommandResult<()> {
    let sheet = workbook.add_worksheet();
    sheet.set_name("导入异常").map_err(xerr)?;
    sheet.write_string(0, 0, DISCLAIMER).map_err(xerr)?;
    let headers = ["快照文件", "状态", "批次", "卷", "导入时间", "原因"];
    for (col, header) in headers.iter().enumerate() {
        sheet.write_string_with_format(1, col as u16, *header, bold).map_err(xerr)?;
    }
    for (offset, record) in exceptions.iter().enumerate() {
        let row = 2 + offset as u32;
        sheet.write_string(row, 0, &record.file).map_err(xerr)?;
        sheet.write_string(row, 1, &record.status).map_err(xerr)?;
        sheet.write_string(row, 2, record.batch_id.as_deref().unwrap_or("")).map_err(xerr)?;
        sheet.write_string(row, 3, record.paper_id.as_deref().unwrap_or("")).map_err(xerr)?;
        sheet.write_string(row, 4, &record.imported_at).map_err(xerr)?;
        sheet.write_string_with_format(row, 5, record.error.as_deref().unwrap_or(""), wrap).map_err(xerr)?;
    }
    sheet.set_column_width(0, 48.0).map_err(xerr)?;
    sheet.set_column_width(5, 60.0).map_err(xerr)?;
    Ok(())
}

fn paper_text(paper_id: &str) -> String {
    if paper_id.is_empty() { "旧套题".to_string() } else { paper_id.to_string() }
}

fn write_json_sheet(workbook:&mut Workbook,name:&str,rows:&[serde_json::Value],n:usize)->CommandResult<()> {
    let sheet=workbook.add_worksheet();sheet.set_name(name).map_err(xerr)?;
    sheet.write_string(0,0,format!("{name}（N={n}）")).map_err(xerr)?;
    sheet.write_string(1,0,DISCLAIMER).map_err(xerr)?;
    let columns:Vec<String>=rows.first().and_then(serde_json::Value::as_object).map(|o|o.keys().cloned().collect()).unwrap_or_default();
    for (i,c) in columns.iter().enumerate(){sheet.write_string(3,i as u16,c).map_err(xerr)?;}
    for (i,row) in rows.iter().enumerate(){for (j,col) in columns.iter().enumerate(){
        let value=&row[col];
        if let Some(n)=value.as_f64(){sheet.write_number(i as u32+4,j as u16,n).map_err(xerr)?;}
        else {let text=value.as_str().map(str::to_string).unwrap_or_else(||if value.is_null(){String::new()}else{value.to_string()});sheet.write_string(i as u32+4,j as u16,text).map_err(xerr)?;}
    }}Ok(())
}

/// Record the export attempt before touching the destination. Even an I/O failure
/// or a later audit failure leaves a durable record of who initiated the export.
pub fn write_export_audited(conn:&Connection,batch_id:&str,paper_id:&str,path:&Path,operator:&str)->CommandResult<ExportSummary>{
    use serde_json::{json,Value};
    super::review::audit(conn,operator,"export_xlsx_requested","paper",paper_id,&Value::Null,&json!({"batchId":batch_id,"path":path}),"导出机构模拟测试成绩")?;
    match write_export(conn,batch_id,paper_id,path){
        Ok(summary)=>{
            super::review::audit(conn,operator,"export_xlsx","paper",paper_id,&Value::Null,&json!({"batchId":batch_id,"path":path,"n":summary.n}),"导出完成")?;
            Ok(summary)
        },
        Err(error)=>{
            let _=super::review::audit(conn,operator,"export_xlsx_failed","paper",paper_id,&Value::Null,&json!({"path":path,"error":error}),"导出失败");
            Err(error)
        }
    }
}
