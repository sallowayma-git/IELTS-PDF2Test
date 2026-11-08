//! 只读查询：批次总览、单次考试、单个学生、单道题（契约之上的统计口径都在这里）。
//!
//! 口径：
//! - 有效计分（valid）：已交卷、有阅读套题记录、篇数 >= 套题 total_passages、每篇 gradable 且满分点数 > 0。
//!   不满足的 attempt 不进入任何统计，但在考试页的「不计入」列表里说明原因。
//! - 原始分 = 各篇 score_info.correct 之和（学生端按 weight 加总），满分 = 各篇 score_info.total 之和。
//! - 题型正确率的分母 = 得分点数（答案行 weight 之和），不是题目个数。
//! - 单题 N = 参与统计的有效 attempt 中该题有判分记录的人数；N < 30 只显示人数，不下「难题」结论。

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, FixedOffset};
use rusqlite::Connection;
use serde::Serialize;

use super::import::{list_failed_imports, last_import_at, ImportRecord};
use super::status::BeaconScan;
use super::{display_answer, passage_label, ratio, DISCLAIMER, HARD_CONCLUSION_MIN_N};
use crate::CommandResult;

// ───────── 原始记录（从分析库读出，不做判分） ─────────

#[derive(Debug, Clone)]
pub struct PassageRec {
    pub passage_index: i64,
    pub category: Option<String>,
    pub title: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub duration_sec: Option<i64>,
    pub gradable: bool,
    pub correct: f64,
    pub total: f64,
}

#[derive(Debug, Clone)]
pub struct AnswerRec {
    pub score: f64,
    pub passage_index: i64,
    pub question_id: String,
    pub display_label: Option<String>,
    pub question_kind: Option<String>,
    pub user_answer_json: Option<String>,
    pub correct_answer_json: Option<String>,
    pub normalized_user_answer_json: Option<String>,
    pub is_correct: Option<i64>,
    pub weight: f64,
    pub elapsed_ms: i64,
    pub change_count: i64,
    pub visit_count: i64,
}

#[derive(Debug, Clone)]
pub struct WritingRec {
    pub task_type: Option<String>,
    pub word_count: Option<i64>,
    pub duration_sec: Option<i64>,
    pub submitted_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AttemptRec {
    pub attempt_id: String,
    pub batch_id: String,
    pub paper_id: String,
    pub candidate_id: String,
    pub candidate_name: String,
    pub submitted_at: Option<String>,
    pub has_suite: bool,
    pub total_passages: i64,
    pub passages: Vec<PassageRec>,
    pub answers: Vec<AnswerRec>,
    pub writings: Vec<WritingRec>,
    /// None = 有效计分；Some = 不计入的原因。
    pub invalid_reason: Option<String>,
}

impl AttemptRec {
    pub fn is_valid(&self) -> bool {
        self.invalid_reason.is_none()
    }
    pub fn raw_score(&self) -> f64 {
        self.passages.iter().map(|p| p.correct).sum()
    }
    pub fn raw_total(&self) -> f64 {
        self.passages.iter().map(|p| p.total).sum()
    }
    fn passage(&self, index: i64) -> Option<&PassageRec> {
        self.passages.iter().find(|p| p.passage_index == index)
    }
}

pub enum Scope<'a> {
    All,
    Group { batch_id: &'a str, paper_id: &'a str },
    Attempt(&'a str),
}

/// 读取已交卷的 attempt 及其子记录，并算出有效性。
pub fn load_attempts(conn: &Connection, scope: &Scope) -> CommandResult<Vec<AttemptRec>> {
    load_attempts_with_mode(conn,scope,false)
}
pub fn load_attempts_with_mode(conn: &Connection, scope: &Scope, original: bool) -> CommandResult<Vec<AttemptRec>> {
    let mut attempts: Vec<AttemptRec> = Vec::new();
    {
        let mut statement = conn
            .prepare(
                "SELECT attempt_id, terminal_id, batch_id, paper_id, candidate_id, candidate_name, submitted_at
                 FROM attempts WHERE status = 'submitted' ORDER BY candidate_id, attempt_id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(AttemptRec {
                    attempt_id: row.get(0)?,
                    batch_id: row.get(2)?,
                    paper_id: row.get(3)?,
                    candidate_id: row.get(4)?,
                    candidate_name: row.get(5)?,
                    submitted_at: row.get(6)?,
                    has_suite: false,
                    total_passages: 0,
                    passages: Vec::new(),
                    answers: Vec::new(),
                    writings: Vec::new(),
                    invalid_reason: None,
                })
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            attempts.push(row.map_err(|error| error.to_string())?);
        }
    }
    attempts.retain(|attempt| match scope {
        Scope::All => true,
        Scope::Group { batch_id, paper_id } => attempt.batch_id == *batch_id && attempt.paper_id == *paper_id,
        Scope::Attempt(id) => attempt.attempt_id == *id,
    });
    if attempts.is_empty() {
        return Ok(attempts);
    }
    let ids: HashSet<String> = attempts.iter().map(|attempt| attempt.attempt_id.clone()).collect();
    let mut index: HashMap<String, usize> = HashMap::new();
    for (position, attempt) in attempts.iter().enumerate() {
        index.insert(attempt.attempt_id.clone(), position);
    }

    let mut statement = conn
        .prepare("SELECT attempt_id, total_passages FROM suites")
        .map_err(|error| error.to_string())?;
    let suites = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
        .map_err(|error| error.to_string())?;
    for suite in suites {
        let (attempt_id, total) = suite.map_err(|error| error.to_string())?;
        if let Some(&position) = index.get(&attempt_id).filter(|_| ids.contains(&attempt_id)) {
            attempts[position].has_suite = true;
            attempts[position].total_passages = attempts[position].total_passages.max(total);
        }
    }

    let mut statement = conn
        .prepare(
            "SELECT attempt_id, passage_index, category, title, started_at, ended_at, duration_sec, gradable, correct, total
             FROM passages ORDER BY attempt_id, passage_index",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                PassageRec {
                    passage_index: row.get(1)?,
                    category: row.get(2)?,
                    title: row.get(3)?,
                    started_at: row.get(4)?,
                    ended_at: row.get(5)?,
                    duration_sec: row.get(6)?,
                    gradable: row.get::<_, i64>(7)? != 0,
                    correct: row.get(8)?,
                    total: row.get(9)?,
                },
            ))
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (attempt_id, passage) = row.map_err(|error| error.to_string())?;
        if let Some(&position) = index.get(&attempt_id) {
            attempts[position].passages.push(passage);
        }
    }

    let mut statement = conn
        .prepare(
            "SELECT attempt_id, passage_index, question_id, display_label, question_kind, user_answer_json,
                    correct_answer_json, normalized_user_answer_json, is_correct, weight, elapsed_ms,
                    change_count, visit_count, COALESCE(score,is_correct*weight,0)
             FROM answers ORDER BY attempt_id, passage_index, question_id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                AnswerRec {
                    score: row.get(13)?,
                    passage_index: row.get(1)?,
                    question_id: row.get(2)?,
                    display_label: row.get(3)?,
                    question_kind: row.get(4)?,
                    user_answer_json: row.get(5)?,
                    correct_answer_json: row.get(6)?,
                    normalized_user_answer_json: row.get(7)?,
                    is_correct: row.get(8)?,
                    weight: row.get(9)?,
                    elapsed_ms: row.get(10)?,
                    change_count: row.get(11)?,
                    visit_count: row.get(12)?,
                },
            ))
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (attempt_id, answer) = row.map_err(|error| error.to_string())?;
        if let Some(&position) = index.get(&attempt_id) {
            attempts[position].answers.push(answer);
        }
    }

    let mut statement = conn
        .prepare("SELECT attempt_id, task_type, word_count, duration_sec, submitted_at FROM writings")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                WritingRec {
                    task_type: row.get(1)?,
                    word_count: row.get(2)?,
                    duration_sec: row.get(3)?,
                    submitted_at: row.get(4)?,
                },
            ))
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (attempt_id, writing) = row.map_err(|error| error.to_string())?;
        if let Some(&position) = index.get(&attempt_id) {
            attempts[position].writings.push(writing);
        }
    }

    for attempt in &mut attempts {
        attempt.invalid_reason = validity(attempt);
        let decision: (i64,i64,Option<String>)=conn.query_row("SELECT counted,counted_override,exclude_reason FROM attempts WHERE attempt_id=?1",[&attempt.attempt_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
        if decision.1==1 && decision.0==0 {attempt.invalid_reason=Some(decision.2.unwrap_or_else(||"教师排除统计".into()));}
        if !original {
            let overrides=super::snapshot_v2::rows(conn,"SELECT * FROM score_overrides WHERE attempt_id=? AND modality='reading' AND revoked_at IS NULL",&[&attempt.attempt_id])?;
            for item in overrides {
                let index=item["passage_index"].as_i64().unwrap_or_default();
                let slot=item["response_slot_id"].as_str().unwrap_or_default();
                let delta=item["new_score"].as_f64().unwrap_or_default()-item["original_score"].as_f64().unwrap_or_default();
                if let Some(p)=attempt.passages.iter_mut().find(|p|p.passage_index==index){p.correct+=delta;}
                if let Some(a)=attempt.answers.iter_mut().find(|a|a.passage_index==index && a.question_id==slot){a.is_correct=item["new_is_correct"].as_i64();a.score=item["new_score"].as_f64().unwrap_or_default();}
            }
        }
    }
    Ok(attempts)
}

fn validity(attempt: &AttemptRec) -> Option<String> {
    if !attempt.has_suite {
        return Some("无阅读作答记录".into());
    }
    let need = attempt.total_passages.max(1);
    let have = attempt.passages.len() as i64;
    if have < need {
        return Some(format!("篇数不足（{have}/{need}）"));
    }
    if attempt.passages.iter().any(|passage| !passage.gradable || passage.total <= 0.0) {
        return Some("存在未计分的篇章".into());
    }
    None
}

// ───────── 批次总览 ─────────

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BatchRow {
    pub batch_id: String,
    pub paper_id: String,
    /// 来自状态信标（实时）。
    pub checked_in: usize,
    pub in_progress: usize,
    pub submitted_beacon: usize,
    pub voided: usize,
    /// 来自分析库（已导入快照）。
    pub imported: usize,
    pub valid_scored: usize,
    pub import_exceptions: usize,
    pub last_submitted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IdentityAttempt {
    pub attempt_id: String,
    pub candidate_id: String,
    pub candidate_name: String,
    pub batch_id: String,
    pub paper_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IdentityIssue {
    /// same_id_diff_name：同一卡号出现不同姓名；same_name_diff_id：同名不同卡号。
    pub kind: String,
    pub candidate_id: String,
    pub candidate_name: String,
    pub attempts: Vec<IdentityAttempt>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub batches: Vec<BatchRow>,
    pub exceptions: Vec<ImportRecord>,
    pub identity_issues: Vec<IdentityIssue>,
    pub beacon_errors: Vec<String>,
    pub beacon_count: usize,
    pub last_import_at: Option<String>,
    pub disclaimer: String,
}

pub fn build_overview(conn: &Connection, beacons: &BeaconScan) -> CommandResult<Overview> {
    let mut groups: BTreeMap<(String, String), BatchRow> = BTreeMap::new();
    for beacon in &beacons.beacons {
        let row = groups
            .entry((beacon.batch_id.clone(), beacon.paper_id.clone()))
            .or_insert_with(|| empty_row(&beacon.batch_id, &beacon.paper_id));
        match beacon.status.as_str() {
            "checked_in" => row.checked_in += 1,
            "in_progress" => row.in_progress += 1,
            "submitted" => {
                row.submitted_beacon += 1;
                row.last_submitted_at = max_opt(row.last_submitted_at.take(), Some(beacon.updated_at.clone()));
            }
            "failed" => row.voided += 1,
            _ => {}
        }
    }

    let all = load_attempts(conn, &Scope::All)?;
    for attempt in &all {
        let row = groups
            .entry((attempt.batch_id.clone(), attempt.paper_id.clone()))
            .or_insert_with(|| empty_row(&attempt.batch_id, &attempt.paper_id));
        row.imported += 1;
        if attempt.is_valid() {
            row.valid_scored += 1;
        }
        row.last_submitted_at = max_opt(row.last_submitted_at.take(), attempt.submitted_at.clone());
    }

    let exceptions = list_failed_imports(conn)?;
    for exception in &exceptions {
        if let (Some(batch), paper) = (exception.batch_id.clone(), exception.paper_id.clone().unwrap_or_default()) {
            if let Some(row) = groups.get_mut(&(batch, paper)) {
                row.import_exceptions += 1;
            }
        }
    }

    Ok(Overview {
        batches: groups.into_values().collect(),
        exceptions,
        identity_issues: identity_issues(&all),
        beacon_errors: beacons.errors.clone(),
        beacon_count: beacons.beacons.len(),
        last_import_at: last_import_at(conn)?,
        disclaimer: DISCLAIMER.to_string(),
    })
}

fn empty_row(batch: &str, paper: &str) -> BatchRow {
    BatchRow {
        batch_id: batch.to_string(),
        paper_id: paper.to_string(),
        checked_in: 0,
        in_progress: 0,
        submitted_beacon: 0,
        voided: 0,
        imported: 0,
        valid_scored: 0,
        import_exceptions: 0,
        last_submitted_at: None,
    }
}

fn max_opt(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(x), Some(y)) => Some(if x >= y { x } else { y }),
        (x, None) => x,
        (None, y) => y,
    }
}

/// 身份核对：同一 candidateId 对应不同姓名，或同名对应不同 candidateId。
pub fn identity_issues(attempts: &[AttemptRec]) -> Vec<IdentityIssue> {
    let mut issues = Vec::new();
    let mut by_id: BTreeMap<&str, Vec<&AttemptRec>> = BTreeMap::new();
    let mut by_name: BTreeMap<&str, Vec<&AttemptRec>> = BTreeMap::new();
    for attempt in attempts {
        by_id.entry(attempt.candidate_id.as_str()).or_default().push(attempt);
        by_name.entry(attempt.candidate_name.as_str()).or_default().push(attempt);
    }
    for (candidate_id, members) in &by_id {
        let names: HashSet<&str> = members.iter().map(|a| a.candidate_name.as_str()).collect();
        if names.len() > 1 {
            issues.push(IdentityIssue {
                kind: "same_id_diff_name".into(),
                candidate_id: candidate_id.to_string(),
                candidate_name: members[0].candidate_name.clone(),
                attempts: members.iter().map(|a| identity_attempt(a)).collect(),
            });
        }
    }
    for (name, members) in &by_name {
        let ids: HashSet<&str> = members.iter().map(|a| a.candidate_id.as_str()).collect();
        if ids.len() > 1 {
            issues.push(IdentityIssue {
                kind: "same_name_diff_id".into(),
                candidate_id: members[0].candidate_id.clone(),
                candidate_name: name.to_string(),
                attempts: members.iter().map(|a| identity_attempt(a)).collect(),
            });
        }
    }
    issues
}

fn identity_attempt(attempt: &AttemptRec) -> IdentityAttempt {
    IdentityAttempt {
        attempt_id: attempt.attempt_id.clone(),
        candidate_id: attempt.candidate_id.clone(),
        candidate_name: attempt.candidate_name.clone(),
        batch_id: attempt.batch_id.clone(),
        paper_id: attempt.paper_id.clone(),
    }
}

// ───────── 单次考试 ─────────

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExcludedAttempt {
    pub attempt_id: String,
    pub candidate_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScoreSummary {
    pub mean: Option<f64>,
    pub median: Option<f64>,
    pub max: Option<f64>,
    pub min: Option<f64>,
    pub mean_accuracy: Option<f64>,
    pub max_total: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistBin {
    pub from: f64,
    pub to: f64,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PartStat {
    pub passage_index: i64,
    pub label: String,
    pub n: usize,
    pub correct: f64,
    pub total: f64,
    pub accuracy: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KindStat {
    /// 原始题型标识（前端映射中文）；缺失时为空串。
    pub kind: String,
    pub n: usize,
    /// 得分点数（weight 之和），即分母口径。
    pub points: f64,
    pub correct_points: f64,
    pub accuracy: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuestionStat {
    pub passage_index: i64,
    pub passage_label: String,
    pub question_id: String,
    pub display_label: String,
    pub question_kind: String,
    pub n: usize,
    pub correct_count: usize,
    pub accuracy: Option<f64>,
    /// N >= 30 才为 true；否则前端只能显示人数，不得下「难题」结论。
    pub hard_conclusion: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MatrixRow {
    pub attempt_id: String,
    pub candidate_name: String,
    /// 与 columns 一一对应：true 对、false 错、null 无判分记录。
    pub cells: Vec<Option<bool>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudentRow {
    pub attempt_id: String,
    pub candidate_id: String,
    pub candidate_name: String,
    pub raw_score: f64,
    pub raw_total: f64,
    pub accuracy: Option<f64>,
    /// 与 parts 一一对应的该篇得分点数。
    pub part_correct: Vec<Option<f64>>,
    pub submitted_at: Option<String>,
    pub total_duration_sec: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExamView {
    pub batch_id: String,
    pub paper_id: String,
    pub n: usize,
    pub excluded: Vec<ExcludedAttempt>,
    pub hard_conclusion_min_n: usize,
    pub disclaimer: String,
    pub score: ScoreSummary,
    pub histogram: Vec<HistBin>,
    pub parts: Vec<PartStat>,
    pub kinds: Vec<KindStat>,
    pub hard_questions: Vec<QuestionStat>,
    pub questions: Vec<QuestionStat>,
    pub matrix: Vec<MatrixRow>,
    pub students: Vec<StudentRow>,
}

pub fn build_exam_view(conn: &Connection, batch_id: &str, paper_id: &str) -> CommandResult<ExamView> {
    let attempts = load_attempts(conn, &Scope::Group { batch_id, paper_id })?;
    Ok(exam_view_from(batch_id, paper_id, &attempts))
}

pub fn exam_view_from(batch_id: &str, paper_id: &str, attempts: &[AttemptRec]) -> ExamView {
    let valid: Vec<&AttemptRec> = attempts.iter().filter(|a| a.is_valid()).collect();
    let excluded = attempts
        .iter()
        .filter(|a| !a.is_valid())
        .map(|a| ExcludedAttempt {
            attempt_id: a.attempt_id.clone(),
            candidate_name: a.candidate_name.clone(),
            reason: a.invalid_reason.clone().unwrap_or_default(),
        })
        .collect();

    // 原始分统计。
    let scores: Vec<f64> = valid.iter().map(|a| a.raw_score()).collect();
    let totals: Vec<f64> = valid.iter().map(|a| a.raw_total()).collect();
    let max_total = totals.iter().cloned().fold(None, |acc: Option<f64>, v| Some(acc.map_or(v, |m| m.max(v))));
    let sum_scores: f64 = scores.iter().sum();
    let sum_totals: f64 = totals.iter().sum();
    let mean_accuracy = ratio(sum_scores, sum_totals);
    let score = ScoreSummary {
        mean: if scores.is_empty() { None } else { Some(sum_scores / scores.len() as f64) },
        median: median(&mut scores.clone()),
        max: scores.iter().cloned().fold(None, |acc: Option<f64>, v| Some(acc.map_or(v, |m| m.max(v)))),
        min: scores.iter().cloned().fold(None, |acc: Option<f64>, v| Some(acc.map_or(v, |m| m.min(v)))),
        mean_accuracy,
        max_total,
    };

    // 直方图：按原始分向下取整分箱，0..=ceil(满分)。
    let mut histogram = Vec::new();
    if let Some(max) = max_total {
        let top = max.ceil().max(0.0) as usize;
        let mut counts = vec![0usize; top + 1];
        for value in &scores {
            let bin = (value.floor().max(0.0) as usize).min(top);
            counts[bin] += 1;
        }
        for (bin, count) in counts.into_iter().enumerate() {
            histogram.push(HistBin { from: bin as f64, to: (bin + 1) as f64, count });
        }
    }

    // Part 统计（按篇序号）。
    let mut part_labels: BTreeMap<i64, String> = BTreeMap::new();
    let mut part_acc: BTreeMap<i64, (usize, f64, f64)> = BTreeMap::new();
    for attempt in &valid {
        for passage in &attempt.passages {
            part_labels
                .entry(passage.passage_index)
                .or_insert_with(|| passage_label(passage.category.as_deref(), passage.passage_index));
            let entry = part_acc.entry(passage.passage_index).or_insert((0, 0.0, 0.0));
            entry.0 += 1;
            entry.1 += passage.correct;
            entry.2 += passage.total;
        }
    }
    let parts: Vec<PartStat> = part_acc
        .iter()
        .map(|(index, (n, correct, total))| PartStat {
            passage_index: *index,
            label: part_labels.get(index).cloned().unwrap_or_else(|| format!("P{}", index + 1)),
            n: *n,
            correct: *correct,
            total: *total,
            accuracy: ratio(*correct, *total),
        })
        .collect();

    // 题目统计（只统计有效 attempt 的判分行，weight > 0）。
    struct QAcc {
        passage_index: i64,
        question_id: String,
        display_label: String,
        question_kind: String,
        n: usize,
        correct: usize,
        score:f64,
        max_score:f64,
    }
    let mut qacc: BTreeMap<(i64, String), QAcc> = BTreeMap::new();
    let mut kind_acc: BTreeMap<String, (HashSet<String>, f64, f64)> = BTreeMap::new();
    for attempt in &valid {
        for answer in &attempt.answers {
            let Some(graded) = answer.is_correct else { continue };
            if answer.weight <= 0.0 {
                continue;
            }
            let key = (answer.passage_index, answer.question_id.clone());
            let entry = qacc.entry(key).or_insert_with(|| QAcc {
                passage_index: answer.passage_index,
                question_id: answer.question_id.clone(),
                display_label: answer.display_label.clone().unwrap_or_else(|| answer.question_id.clone()),
                question_kind: answer.question_kind.clone().unwrap_or_default(),
                n: 0,
                correct: 0,
                score:0.0,
                max_score:0.0,
            });
            entry.score+=answer.score;
            entry.max_score+=answer.weight;
            entry.n += 1;
            if graded == 1 {
                entry.correct += 1;
            }
            let kind = answer.question_kind.clone().unwrap_or_default();
            let slot = kind_acc.entry(kind).or_insert_with(|| (HashSet::new(), 0.0, 0.0));
            slot.0.insert(attempt.attempt_id.clone());
            slot.1 += answer.weight;
            slot.2 += answer.score;
        }
    }
    let part_label_of = |index: i64| part_labels.get(&index).cloned().unwrap_or_else(|| format!("P{}", index + 1));
    let mut questions: Vec<QuestionStat> = qacc
        .values()
        .map(|q| QuestionStat {
            passage_index: q.passage_index,
            passage_label: part_label_of(q.passage_index),
            question_id: q.question_id.clone(),
            display_label: q.display_label.clone(),
            question_kind: q.question_kind.clone(),
            n: q.n,
            correct_count: q.correct,
            accuracy: ratio(q.score, q.max_score),
            hard_conclusion: q.n >= HARD_CONCLUSION_MIN_N,
        })
        .collect();
    questions.sort_by(|a, b| (a.passage_index, a.question_id.len(), &a.question_id).cmp(&(b.passage_index, b.question_id.len(), &b.question_id)));
    let mut hard_questions = questions.clone();
    hard_questions.sort_by(|a, b| {
        let av = a.accuracy.unwrap_or(1.0);
        let bv = b.accuracy.unwrap_or(1.0);
        av.partial_cmp(&bv).unwrap_or(std::cmp::Ordering::Equal).then(b.n.cmp(&a.n))
    });
    hard_questions.truncate(10);

    let mut kinds: Vec<KindStat> = kind_acc
        .into_iter()
        .map(|(kind, (students, points, correct_points))| KindStat {
            kind,
            n: students.len(),
            points,
            correct_points,
            accuracy: ratio(correct_points, points),
        })
        .collect();
    kinds.sort_by(|a, b| b.points.partial_cmp(&a.points).unwrap_or(std::cmp::Ordering::Equal));

    // 学生×题目矩阵与学生表。
    let matrix_cols: Vec<(i64, String)> = questions.iter().map(|q| (q.passage_index, q.question_id.clone())).collect();
    let mut students_sorted: Vec<&AttemptRec> = valid.clone();
    students_sorted.sort_by(|a, b| a.candidate_id.cmp(&b.candidate_id).then(a.attempt_id.cmp(&b.attempt_id)));
    let part_indices: Vec<i64> = parts.iter().map(|p| p.passage_index).collect();
    let matrix: Vec<MatrixRow> = students_sorted
        .iter()
        .map(|attempt| {
            let by_key: HashMap<(i64, &str), &AnswerRec> = attempt
                .answers
                .iter()
                .map(|answer| ((answer.passage_index, answer.question_id.as_str()), answer))
                .collect();
            MatrixRow {
                attempt_id: attempt.attempt_id.clone(),
                candidate_name: attempt.candidate_name.clone(),
                cells: matrix_cols
                    .iter()
                    .map(|(index, qid)| {
                        by_key
                            .get(&(*index, qid.as_str()))
                            .and_then(|answer| answer.is_correct.map(|v| v == 1))
                    })
                    .collect(),
            }
        })
        .collect();
    let students: Vec<StudentRow> = students_sorted
        .iter()
        .map(|attempt| StudentRow {
            attempt_id: attempt.attempt_id.clone(),
            candidate_id: attempt.candidate_id.clone(),
            candidate_name: attempt.candidate_name.clone(),
            raw_score: attempt.raw_score(),
            raw_total: attempt.raw_total(),
            accuracy: ratio(attempt.raw_score(), attempt.raw_total()),
            part_correct: part_indices.iter().map(|index| attempt.passage(*index).map(|p| p.correct)).collect(),
            submitted_at: attempt.submitted_at.clone(),
            total_duration_sec: {
                let durations: Vec<i64> = attempt.passages.iter().filter_map(|p| p.duration_sec).collect();
                if durations.is_empty() { None } else { Some(durations.iter().sum()) }
            },
        })
        .collect();

    ExamView {
        batch_id: batch_id.to_string(),
        paper_id: paper_id.to_string(),
        n: valid.len(),
        excluded,
        hard_conclusion_min_n: HARD_CONCLUSION_MIN_N,
        disclaimer: DISCLAIMER.to_string(),
        score,
        histogram,
        parts,
        kinds,
        hard_questions,
        questions,
        matrix,
        students,
    }
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 0 { (values[mid - 1] + values[mid]) / 2.0 } else { values[mid] })
}

// ───────── 单个学生 ─────────

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudentPassage {
    pub passage_index: i64,
    pub label: String,
    pub title: Option<String>,
    pub correct: f64,
    pub total: f64,
    pub accuracy: Option<f64>,
    pub duration_sec: Option<i64>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    /// 与其他篇的作答区间重叠：用时可能重叠。
    pub overlap: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WrongAnswer {
    pub passage_index: i64,
    pub passage_label: String,
    pub question_id: String,
    pub display_label: String,
    pub question_kind: String,
    pub user_answer: String,
    pub correct_answer: String,
    pub weight: f64,
    pub elapsed_ms: i64,
    pub change_count: i64,
    pub visit_count: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WritingRow {
    pub task_type: String,
    pub word_count: Option<i64>,
    pub duration_sec: Option<i64>,
    pub submitted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudentView {
    pub attempt_id: String,
    pub batch_id: String,
    pub paper_id: String,
    pub candidate_id: String,
    pub candidate_name: String,
    pub submitted_at: Option<String>,
    pub valid: bool,
    pub invalid_reason: Option<String>,
    pub raw_score: f64,
    pub raw_total: f64,
    pub accuracy: Option<f64>,
    pub passages: Vec<StudentPassage>,
    pub overlap_note: Option<String>,
    pub wrong_answers: Vec<WrongAnswer>,
    pub writings: Vec<WritingRow>,
    pub disclaimer: String,
}

pub fn build_student_view(conn: &Connection, attempt_id: &str) -> CommandResult<StudentView> {
    let attempts = load_attempts(conn, &Scope::Attempt(attempt_id))?;
    let attempt = attempts
        .into_iter()
        .next()
        .ok_or_else(|| format!("ATTEMPT_NOT_FOUND:{attempt_id}"))?;
    Ok(student_view_from(&attempt))
}

pub fn student_view_from(attempt: &AttemptRec) -> StudentView {
    let intervals: Vec<Option<(DateTime<FixedOffset>, DateTime<FixedOffset>)>> = attempt
        .passages
        .iter()
        .map(|p| {
            let start = p.started_at.as_deref().and_then(|t| DateTime::parse_from_rfc3339(t).ok())?;
            let end = p.ended_at.as_deref().and_then(|t| DateTime::parse_from_rfc3339(t).ok())?;
            Some((start, end))
        })
        .collect();
    let mut overlap = vec![false; attempt.passages.len()];
    for i in 0..attempt.passages.len() {
        for j in (i + 1)..attempt.passages.len() {
            if let (Some((a_start, a_end)), Some((b_start, b_end))) = (intervals[i], intervals[j]) {
                if a_start < b_end && b_start < a_end {
                    overlap[i] = true;
                    overlap[j] = true;
                }
            }
        }
    }
    let passages: Vec<StudentPassage> = attempt
        .passages
        .iter()
        .enumerate()
        .map(|(position, p)| StudentPassage {
            passage_index: p.passage_index,
            label: passage_label(p.category.as_deref(), p.passage_index),
            title: p.title.clone(),
            correct: p.correct,
            total: p.total,
            accuracy: ratio(p.correct, p.total),
            duration_sec: p.duration_sec,
            started_at: p.started_at.clone(),
            ended_at: p.ended_at.clone(),
            overlap: overlap[position],
        })
        .collect();
    let mut wrong_answers: Vec<WrongAnswer> = attempt
        .answers
        .iter()
        .filter(|answer| answer.is_correct == Some(0))
        .map(|answer| WrongAnswer {
            passage_index: answer.passage_index,
            passage_label: attempt
                .passage(answer.passage_index)
                .map(|p| passage_label(p.category.as_deref(), p.passage_index))
                .unwrap_or_else(|| format!("P{}", answer.passage_index + 1)),
            question_id: answer.question_id.clone(),
            display_label: answer.display_label.clone().unwrap_or_else(|| answer.question_id.clone()),
            question_kind: answer.question_kind.clone().unwrap_or_default(),
            user_answer: display_answer(answer.user_answer_json.as_deref()),
            correct_answer: display_answer(answer.correct_answer_json.as_deref()),
            weight: answer.weight,
            elapsed_ms: answer.elapsed_ms,
            change_count: answer.change_count,
            visit_count: answer.visit_count,
        })
        .collect();
    wrong_answers.sort_by(|a, b| (a.passage_index, a.question_id.len(), &a.question_id).cmp(&(b.passage_index, b.question_id.len(), &b.question_id)));
    let writings = attempt
        .writings
        .iter()
        .map(|w| WritingRow {
            task_type: w.task_type.clone().unwrap_or_default(),
            word_count: w.word_count,
            duration_sec: w.duration_sec,
            submitted_at: w.submitted_at.clone(),
        })
        .collect();
    StudentView {
        attempt_id: attempt.attempt_id.clone(),
        batch_id: attempt.batch_id.clone(),
        paper_id: attempt.paper_id.clone(),
        candidate_id: attempt.candidate_id.clone(),
        candidate_name: attempt.candidate_name.clone(),
        submitted_at: attempt.submitted_at.clone(),
        valid: attempt.is_valid(),
        invalid_reason: attempt.invalid_reason.clone(),
        raw_score: attempt.raw_score(),
        raw_total: attempt.raw_total(),
        accuracy: ratio(attempt.raw_score(), attempt.raw_total()),
        passages,
        overlap_note: if overlap.iter().any(|v| *v) {
            Some("部分篇章的作答时间区间重叠，用时可能重叠，仅供参考。".into())
        } else {
            None
        },
        wrong_answers,
        writings,
        disclaimer: DISCLAIMER.to_string(),
    }
}

// ───────── 单道题 ─────────

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnswerBucket {
    pub answer: String,
    pub count: usize,
    pub share: Option<f64>,
    /// 该答案是否为正确答案（判分行的 is_correct 决定；同一归一化答案结论一致）。
    pub is_correct: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuestionView {
    pub batch_id: String,
    pub paper_id: String,
    pub passage_index: i64,
    pub passage_label: String,
    pub question_id: String,
    pub display_label: String,
    pub question_kind: String,
    pub n: usize,
    pub correct_count: usize,
    pub accuracy: Option<f64>,
    pub hard_conclusion: bool,
    pub correct_answer: String,
    pub avg_elapsed_ms: Option<f64>,
    pub avg_change_count: Option<f64>,
    pub distribution: Vec<AnswerBucket>,
    pub disclaimer: String,
}

pub fn build_question_view(
    conn: &Connection,
    batch_id: &str,
    paper_id: &str,
    passage_index: i64,
    question_id: &str,
) -> CommandResult<QuestionView> {
    let attempts = load_attempts(conn, &Scope::Group { batch_id, paper_id })?;
    let valid: Vec<&AttemptRec> = attempts.iter().filter(|a| a.is_valid()).collect();
    let mut n = 0usize;
    let mut correct = 0usize;
    let mut score_sum=0.0;
    let mut max_sum=0.0;
    let mut elapsed_sum = 0f64;
    let mut change_sum = 0f64;
    let mut display_label: Option<String> = None;
    let mut question_kind: Option<String> = None;
    let mut correct_answer: Option<String> = None;
    let mut buckets: BTreeMap<String, (usize, Option<bool>)> = BTreeMap::new();
    for attempt in &valid {
        for answer in attempt.answers.iter().filter(|a| a.passage_index == passage_index && a.question_id == question_id) {
            display_label = display_label.or_else(|| answer.display_label.clone());
            question_kind = question_kind.or_else(|| answer.question_kind.clone());
            if correct_answer.is_none() {
                correct_answer = Some(display_answer(answer.correct_answer_json.as_deref()));
            }
            let Some(graded) = answer.is_correct else { continue };
            if answer.weight <= 0.0 {
                continue;
            }
            n += 1;
            if graded == 1 {
                correct += 1;
            }
            score_sum+=answer.score;max_sum+=answer.weight;
            elapsed_sum += answer.elapsed_ms as f64;
            change_sum += answer.change_count as f64;
            let text = display_answer(answer.normalized_user_answer_json.as_deref());
            let bucket = buckets.entry(text).or_insert((0, None));
            bucket.0 += 1;
            bucket.1 = Some(graded == 1);
        }
    }
    let mut distribution: Vec<AnswerBucket> = buckets
        .into_iter()
        .map(|(answer, (count, is_correct))| AnswerBucket {
            answer,
            count,
            share: ratio(count as f64, n as f64),
            is_correct,
        })
        .collect();
    distribution.sort_by(|a, b| b.count.cmp(&a.count).then(a.answer.cmp(&b.answer)));
    let passage_label_text = valid
        .iter()
        .find_map(|attempt| attempt.passage(passage_index).map(|p| passage_label(p.category.as_deref(), p.passage_index)))
        .unwrap_or_else(|| format!("P{}", passage_index + 1));
    Ok(QuestionView {
        batch_id: batch_id.to_string(),
        paper_id: paper_id.to_string(),
        passage_index,
        passage_label: passage_label_text,
        question_id: question_id.to_string(),
        display_label: display_label.unwrap_or_else(|| question_id.to_string()),
        question_kind: question_kind.unwrap_or_default(),
        n,
        correct_count: correct,
        accuracy: ratio(score_sum, max_sum),
        hard_conclusion: n >= HARD_CONCLUSION_MIN_N,
        correct_answer: correct_answer.unwrap_or_else(|| "（未作答）".into()),
        avg_elapsed_ms: ratio(elapsed_sum, n as f64),
        avg_change_count: ratio(change_sum, n as f64),
        distribution,
        disclaimer: DISCLAIMER.to_string(),
    })
}
