"""复现检查：把每处修复**退回旧行为**，确认对应的回归测试真的会红。

不做这件事的话，新增的测试可能只是「照着实现写的绿灯」——那比没有测试更危险。
每个 case 都要求：改动前（旧行为）测试 FAIL，改动后（当前实现）测试 PASS。

跑法（在仓库根目录）：

    python scripts/e2e/lib/repro-check-cloud-repair-tests.py

退出码 0 = 每条用例在旧行为下都变红、且源文件未被留在变异状态。

## 为什么这份脚本和上一版不一样

上一版把源码就地改写在 finally 里还原，结果脚本被中途打断（两次），源码被留在
**变异状态**，而且它读到的「原始内容」本身就是上一次残留的变异版本——后续所有结论
全部作废。所以现在**不再依赖 git**：

  1. 开跑前把每个待变异源文件**原样快照**到 `.scratch/repro-check-snapshot/`。
     不要求工作区干净——本轮改动往往还没提交，`git checkout --` 会把成果一起抹掉。
  2. 「原始内容」和「还原」都只认这份快照，与 git 状态完全解耦。
  3. 每个 case 跑完立刻还原；`finally` 里再兜一次；结束时逐字节比对源文件与快照。

另外两个必须显式核对的点，否则整份证据是假的：
  1. 测试**真的跑了一个**。`cargo test --lib -- <name> --exact` 用短名匹配不到任何测试，
     退出码是 0——「没跑」会被读成「通过」。所以这里解析 `test result:` 行，
     `ran != 1` 一律记 ERROR。
  2. 变红的理由是**断言失败**，不是编译不过。编译错误也返回非 0，但它证明不了行为差异。

这份脚本 2026-09-19 抓到过一条真问题：`read_source` 的 PDF 分支没有测试覆盖到「接线」
（测试直接调助手函数 `attach_page_texts`，把调用点删掉照样绿）。修法是把它抽成
`pdf_read_source_response` 再驱动，见 `cloud_repair/mod.rs`。
"""

import os
import re
import subprocess
import sys

SRC = "src-tauri/src/cloud_repair/mod.rs"
TOOLS_SRC = "src-tauri/src/cloud_repair/tools.rs"

ENV = dict(os.environ)
ENV["CARGO_HTTP_PROXY"] = "http://127.0.0.1:49870"

TEST_PREFIX = "cloud_repair::tests::"

# (说明, 完整测试名, 源文件, [(旧片段, 旧行为片段), ...])
CASES = [
    (
        "终态保证：循环内部失败返回 Err（旧行为）",
        "every_exit_path_returns_a_terminal_report_and_never_leaves_running",
        SRC,
        [
            (
                "            Err(error) => {\n"
                "                last_error = Some(error);\n"
                "                status = REPAIR_STATUS_UNAVAILABLE;\n"
                "                break;\n"
                "            }\n"
                "        };\n"
                "        let call = match parse_tool_call(&raw) {",
                "            Err(error) => {\n"
                "                return Err(error);\n"
                "            }\n"
                "        };\n"
                "        let call = match parse_tool_call(&raw) {",
            )
        ],
    ),
    (
        "budget_exhausted 用 finish_note.is_none() 判定（旧行为）",
        "a_model_that_finishes_without_a_note_is_not_reported_as_budget_exhausted",
        SRC,
        [
            (
                "if rounds >= request.max_rounds && status == REPAIR_STATUS_COMPLETED && !finished {",
                "if rounds >= request.max_rounds && status == REPAIR_STATUS_COMPLETED && finish_note.is_none() {",
            )
        ],
    ),
    (
        "last_error 写入可恢复错误 + 成功时不清空（旧行为）",
        "a_recovered_tool_error_does_not_leave_a_last_error_on_a_successful_run",
        SRC,
        [
            (
                '"errors": [error],\n                }));\n                continue;',
                '"errors": [error],\n                }));\n                last_error = Some(error);\n                continue;',
            ),
            (
                "        last_error: if status == REPAIR_STATUS_COMPLETED {\n"
                "            None\n"
                "        } else {\n"
                "            last_error\n"
                "        },",
                "        last_error: last_error,",
            ),
        ],
    ),
    (
        "adjudicated_count = rulings.len()（旧行为）",
        "adjudicated_count_only_counts_rulings_that_still_hold",
        SRC,
        [
            (
                "fn effective_adjudicated_count(canonical: &Value, candidate: &Value, rulings: &[Value]) -> usize {\n"
                "    candidate_differences(canonical, candidate)",
                "fn effective_adjudicated_count(canonical: &Value, candidate: &Value, rulings: &[Value]) -> usize {\n"
                "    if !rulings.is_empty() { return rulings.len(); }\n"
                "    candidate_differences(canonical, candidate)",
            )
        ],
    ),
    (
        "裁定指纹只看摘要（旧行为：task_group 用 group_index_entry）",
        "a_ruling_stops_holding_once_the_content_it_depended_on_changes",
        SRC,
        [
            (
                "_ => {\n"
                "            let group = group_containing(&|group: &Value| {\n"
                '                group.get("taskId").and_then(Value::as_str) == Some(target_id)\n'
                "            });\n"
                "            canonical_json(&group)\n"
                "        }",
                "_ => {\n"
                "            let group = group_containing(&|group: &Value| {\n"
                '                group.get("taskId").and_then(Value::as_str) == Some(target_id)\n'
                "            });\n"
                "            canonical_json(&group_index_entry(&group))\n"
                "        }",
            )
        ],
    ),
    (
        "裁定指纹不看选项库（旧行为：slot 只用摘要）",
        "a_ruling_grounded_in_the_option_bank_dies_when_the_option_bank_changes",
        SRC,
        [('                "group": owning,', '                "group": group_index_entry(&owning),')],
    ),
    (
        "阻断任务的空目标 + 无兜底动作（旧行为）",
        "a_blocking_task_always_carries_a_real_action",
        SRC,
        [
            (
                '"action": if target_ids.is_empty() { "review_source" } else { "fix_blocking_issue" },',
                '"action": "fix_blocking_issue",',
            )
        ],
    ),
    (
        "文档级问题按 (code, 空目标) 折叠（旧行为）",
        "two_different_document_level_issues_are_not_collapsed_into_one",
        SRC,
        [
            (
                "        let identity = match target_ids.first() {\n"
                "            Some(target_id) => target_id.clone(),\n"
                "            None => issue\n"
                '                .get("issueId")\n'
                "                .and_then(Value::as_str)\n"
                '                .unwrap_or("")\n'
                "                .to_string(),\n"
                "        };",
                "        let identity = target_ids.first().cloned().unwrap_or_default();",
            )
        ],
    ),
    (
        "跨源去重不共享「补答案」族（旧行为）",
        "one_missing_answer_is_reported_once_at_the_most_severe_level",
        SRC,
        [
            (
                '"ANSWER_KEY_MISSING_SLOT" | "ANSWER_MISSING" | "ANSWER_KEY_UNRESOLVED" => {\n'
                '            "answer".to_string()\n'
                "        }",
                '"ANSWER_KEY_MISSING_SLOT" | "ANSWER_MISSING" | "ANSWER_KEY_UNRESOLVED" => {\n'
                '            format!("issue:{code}")\n'
                "        }",
            )
        ],
    ),
    (
        "去重键忽略「修理种类」（旧行为：只按目标合并）",
        "different_repairs_on_the_same_target_are_not_merged",
        SRC,
        [("    (target, family)\n}", "    let _ = family;\n    (target, String::new())\n}")],
    ),
    (
        "read_source 不回原文文本（旧行为：PDF 只回页图）",
        "read_source_pages_carry_the_text_layer_extracted_from_the_original_file",
        SRC,
        [("    let pages = attach_page_texts(&selected, page_texts);", "    let pages = selected;")],
    ),
]

SUMMARY = re.compile(r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed")


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, cwd=".")


# 关键安全设计：**不用 git 还原**。
# 上一版的教训是「就地改写 + finally 还原」，脚本被打断就留下变异源码。
# 这里再加一条：当前实现**尚未提交**（工作区有本轮改动），`git checkout --` 会把
# 本轮成果一并抹掉。所以改成——开跑前把每个源文件原样快照到临时目录，之后
# 「原始内容」和「还原」都只认这份快照，与 git 状态完全解耦。
# 快照落在 `.scratch/`（已 gitignore），不会污染 `git status`。
REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
SNAPSHOT_DIR = os.path.join(REPO_ROOT, ".scratch", "repro-check-snapshot")


def snapshot(paths):
    os.makedirs(SNAPSHOT_DIR, exist_ok=True)
    for path in paths:
        with open(path, "rb") as handle:
            data = handle.read()
        with open(os.path.join(SNAPSHOT_DIR, path.replace("/", "__")), "wb") as handle:
            handle.write(data)


def pristine_of(path):
    # 统一换行：源文件在工作区是 CRLF，而锚点里写的是 "\n"。
    # 用 universal newlines 读进来（\r\n -> \n），变异后按 LF 写回；
    # 跑完再从二进制快照还原，字节完全一致。
    with open(os.path.join(SNAPSHOT_DIR, path.replace("/", "__")), "r", encoding="utf-8", newline=None) as handle:
        return handle.read()


def restore(path):
    with open(os.path.join(SNAPSHOT_DIR, path.replace("/", "__")), "rb") as handle:
        data = handle.read()
    with open(path, "wb") as handle:
        handle.write(data)


def unchanged_from_snapshot(paths):
    for path in paths:
        with open(path, "rb") as handle:
            current = handle.read()
        with open(os.path.join(SNAPSHOT_DIR, path.replace("/", "__")), "rb") as handle:
            original = handle.read()
        if current != original:
            return False
    return True


def run_test(full_name):
    proc = subprocess.run(
        ["cargo", "test", "--manifest-path", "src-tauri/Cargo.toml", "--lib", "--", full_name, "--exact"],
        capture_output=True,
        text=True,
        env=ENV,
    )
    stdout, stderr = proc.stdout, proc.stderr
    match = SUMMARY.search(stdout)
    passed = int(match.group(2)) if match else 0
    failed = int(match.group(3)) if match else 0
    compile_error = "error[E" in stderr or "could not compile" in stderr
    reasons = [
        line.strip()
        for line in stdout.splitlines()
        if "assertion" in line or "panicked at" in line or "left:" in line or "right:" in line
    ][:6]
    return {
        "ran": passed + failed,
        "passed": passed,
        "failed": failed,
        "compile_error": compile_error,
        "reasons": reasons,
    }


def main():
    sources = sorted({src for _, _, src, _ in CASES})
    snapshot(sources)
    print("== 步骤 0：已快照待变异源文件 ==", flush=True)
    for src in sources:
        print(f"  {src}", flush=True)
    print(flush=True)

    print("== 步骤 1：确认当前实现下所有用例都是绿的（且真的跑了）==", flush=True)
    baseline_ok = True
    for label, test, _, _ in CASES:
        result = run_test(TEST_PREFIX + test)
        if result["compile_error"]:
            print(f"  [COMPILE-ERROR] {test}", flush=True)
            baseline_ok = False
        elif result["ran"] != 1 or result["passed"] != 1:
            print(
                f"  [NOT-GREEN] {test} ran={result['ran']} passed={result['passed']} failed={result['failed']}",
                flush=True,
            )
            baseline_ok = False
        else:
            print(f"  [PASS] {test}", flush=True)
    print(f"\n基线全部为真绿：{baseline_ok}\n", flush=True)

    print("== 步骤 2：逐条退回旧行为，确认对应用例变红（且是断言失败）==", flush=True)
    results = []
    try:
        for label, test, src, subs in CASES:
            pristine = pristine_of(src)
            mutated = pristine
            unique = True
            for old, new in subs:
                if mutated.count(old) != 1:
                    print(f"  [SKIP] {label}：锚点不唯一（{mutated.count(old)} 处）", flush=True)
                    unique = False
                    break
                mutated = mutated.replace(old, new)
            if not unique:
                results.append((label, "SKIP", []))
                continue
            with open(src, "w", encoding="utf-8", newline="") as handle:
                handle.write(mutated)
            result = run_test(TEST_PREFIX + test)
            restore(src)
            if result["compile_error"]:
                verdict = "COMPILE-ERROR（无效证据）"
            elif result["ran"] != 1:
                verdict = f"NO-TEST-RAN（无效证据，ran={result['ran']}）"
            elif result["failed"] == 1:
                verdict = "RED（符合预期）"
            else:
                verdict = "GREEN（测试没有牙！）"
            results.append((label, verdict, result["reasons"]))
            print(f"  [{verdict}] {label}", flush=True)
            for reason in result["reasons"]:
                print(f"      {reason}", flush=True)
    finally:
        for src in sources:
            restore(src)
        print("\n源文件已从快照还原。", flush=True)

    print("\n== 汇总 ==", flush=True)
    for label, verdict, _ in results:
        print(f"  {verdict:26s} {label}", flush=True)

    intact = unchanged_from_snapshot(sources)
    bad = [r for r in results if r[1] != "RED（符合预期）"]
    print(f"\n结论：{len(results) - len(bad)}/{len(results)} 条用例在旧行为下变红。", flush=True)
    print(f"源文件与快照一致（未被留在变异状态）：{intact}", flush=True)
    return 0 if (not bad and intact) else 1


if __name__ == "__main__":
    sys.exit(main())
