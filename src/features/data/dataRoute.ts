// 「数据」模块的子视图路由（挂在 #/data 下，浏览器前进/后退即可钻取回退）。
//
//   #/data                                   批次总览
//   #/data/exam?batch=<b>&paper=<p>          单次考试（paper 为空串 = 旧套题）
//   #/data/student/<attemptId>               单个学生
//   #/data/question?batch=&paper=&passage=&qid=   单道题

export type DataView =
  | { kind: "batches" }
  | { kind: "exam"; batchId: string; paperId: string }
  | { kind: "student"; attemptId: string }
  | { kind: "question"; batchId: string; paperId: string; passageIndex: number; questionId: string };

const BATCHES: DataView = { kind: "batches" };

function splitHash(hash: string): { path: string[]; query: URLSearchParams } {
  const value = hash.replace(/^#\/?/, "");
  const [pathPart, queryPart = ""] = value.split("?");
  const path = pathPart.split("/").filter(Boolean);
  return { path, query: new URLSearchParams(queryPart) };
}

/** 解析 `#/data/...`。不认识的子路径回到批次总览，不报错。 */
export function parseDataView(hash: string): DataView {
  const { path, query } = splitHash(hash);
  if (path[0] !== "data") return BATCHES;
  const head = path[1];
  if (head === "exam") {
    const batchId = query.get("batch") ?? "";
    if (!batchId) return BATCHES;
    return { kind: "exam", batchId, paperId: query.get("paper") ?? "" };
  }
  if (head === "student" && path[2]) {
    return { kind: "student", attemptId: decodeURIComponent(path[2]) };
  }
  if (head === "question") {
    const batchId = query.get("batch") ?? "";
    const questionId = query.get("qid") ?? "";
    const passageIndex = Number(query.get("passage"));
    if (!batchId || !questionId || !Number.isInteger(passageIndex)) return BATCHES;
    return { kind: "question", batchId, paperId: query.get("paper") ?? "", passageIndex, questionId };
  }
  return BATCHES;
}

/** 生成 hash 路径（不含 `#`）。 */
export function dataViewPath(view: DataView): string {
  switch (view.kind) {
    case "batches":
      return "/data";
    case "exam":
      return `/data/exam?${new URLSearchParams({ batch: view.batchId, paper: view.paperId }).toString()}`;
    case "student":
      return `/data/student/${encodeURIComponent(view.attemptId)}`;
    case "question":
      return `/data/question?${new URLSearchParams({
        batch: view.batchId,
        paper: view.paperId,
        passage: String(view.passageIndex),
        qid: view.questionId
      }).toString()}`;
  }
}
