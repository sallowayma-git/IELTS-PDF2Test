#!/usr/bin/env python3
"""把权威稿与批次行导出成 JSON，用于「数据库前后差异」这一份证据。

为什么单独一个脚本：产品链验收要求交付**真实数据库的前后差异**，而不是界面文字。
Node 侧读 SQLite 需要额外依赖/实验开关，而 Python 的 sqlite3 是标准库，最稳。
本脚本只读不写。

用法：
    python scripts/e2e/lib/dump-authoring-db.py <db> <out.json> [itemId]
"""

import json
import sqlite3
import sys


def main() -> int:
    if len(sys.argv) < 3:
        print("usage: dump-authoring-db.py <db> <out.json> [itemId]", file=sys.stderr)
        return 2
    db_path, out_path = sys.argv[1], sys.argv[2]
    item_id = sys.argv[3] if len(sys.argv) > 3 else None

    con = sqlite3.connect(db_path)
    con.row_factory = sqlite3.Row
    cur = con.cursor()

    out = {"db": db_path, "itemId": item_id, "item": None, "batches": [], "decisions": [], "journal": [], "processingJobs": []}

    where = "WHERE id = ?" if item_id else "ORDER BY id LIMIT 1"
    params = (item_id,) if item_id else ()
    row = cur.execute(
        f"SELECT id, title, status, current_edit_version, canonical_ds_json, updated_at FROM library_items_v2 {where}",
        params,
    ).fetchone()
    if row is not None:
        canonical = json.loads(row["canonical_ds_json"]) if row["canonical_ds_json"] else None
        out["item"] = {
            "id": row["id"],
            "title": row["title"],
            "status": row["status"],
            "editVersion": row["current_edit_version"],
            "updatedAt": row["updated_at"],
            "canonical": canonical,
        }
        item_id = row["id"]
        out["itemId"] = item_id

    if item_id:
        for batch in cur.execute(
            "SELECT * FROM recognition_batches_v1 WHERE library_item_id = ? ORDER BY updated_at",
            (item_id,),
        ).fetchall():
            entry = dict(batch)
            for key in ("stages_json", "repair_json"):
                if entry.get(key):
                    try:
                        entry[key] = json.loads(entry[key])
                    except Exception:  # noqa: BLE001 - 原始文本比「解析失败」更有用
                        pass
            out["batches"].append(entry)

        for decision in cur.execute(
            "SELECT batch_id, decision_id, resolution, code, severity, target_type, target_id, field, status "
            "FROM recognition_decisions_v1 WHERE library_item_id = ? ORDER BY decision_id",
            (item_id,),
        ).fetchall():
            out["decisions"].append(dict(decision))

        for row in cur.execute(
            "SELECT * FROM editor_journal_v1 WHERE library_item_id = ? ORDER BY rowid",
            (item_id,),
        ).fetchall():
            out["journal"].append(dict(row))

    # 处理任务行（跨条目）：`stage` / `retry_count` / `last_error_code` 是「重新识别到底
    # 有没有跑起来」的唯一权威。只读，不解释。
    for row in cur.execute(
        "SELECT id, library_item_id, stage, local_status, cloud_status, reconcile_status, "
        "retry_count, last_error_code, actionable_count, updated_at FROM processing_jobs_v2 "
        "ORDER BY updated_at"
    ).fetchall():
        out["processingJobs"].append(dict(row))

    with open(out_path, "w", encoding="utf-8") as handle:
        json.dump(out, handle, ensure_ascii=False, indent=2)
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
