import { describe, expect, it } from "vitest";
import { dataViewPath, parseDataView } from "./dataRoute";

describe("数据模块子路由", () => {
  it("#/data 落到批次总览", () => {
    expect(parseDataView("#/data")).toEqual({ kind: "batches" });
    expect(parseDataView("#/data/unknown")).toEqual({ kind: "batches" });
  });

  it("考试页保留空套题 paper（旧套题）", () => {
    expect(parseDataView("#/data/exam?batch=B1&paper=")).toEqual({ kind: "exam", batchId: "B1", paperId: "" });
    expect(parseDataView("#/data/exam?batch=B1&paper=paper-A")).toEqual({ kind: "exam", batchId: "B1", paperId: "paper-A" });
  });

  it("缺少必要参数时回退到批次总览", () => {
    expect(parseDataView("#/data/exam?paper=paper-A")).toEqual({ kind: "batches" });
    expect(parseDataView("#/data/question?batch=B1&paper=&passage=x&qid=q1")).toEqual({ kind: "batches" });
  });

  it("学生与题目页可往返生成路径", () => {
    const student = { kind: "student" as const, attemptId: "attempt_01/x" };
    expect(parseDataView(`#${dataViewPath(student)}`)).toEqual(student);
    const question = { kind: "question" as const, batchId: "B 1", paperId: "", passageIndex: 2, questionId: "p2q3" };
    expect(parseDataView(`#${dataViewPath(question)}`)).toEqual(question);
    const exam = { kind: "exam" as const, batchId: "B1", paperId: "" };
    expect(parseDataView(`#${dataViewPath(exam)}`)).toEqual(exam);
  });
});
