# 第三波复核结论与返工任务书（2026-09-23）

> 给执行 agent：这是质量把关方对分支 `wave3-listening`（T1–T5）的复核结果。先读完第 1 节，
> 再按第 3 节逐条返工。原任务书 `docs/handoff/2026-09-22-execution-prompt.md` 的工作约定、边界、回报格式继续有效。

---

## 1. 复核结论

### 1.1 独立复现通过（与回报一致）

| 项 | 回报 | 质量方复跑 |
| --- | --- | --- |
| `cargo test --lib` | 1023 / 0 / 11 | **1023 / 0 / 11** ✅ |
| `npx vitest run` | 397 / 27 files | **397 / 27 files** ✅ |
| `npx tsc --noEmit` | 干净 | **干净** ✅ |
| 阅读真实 Tauri 链 `tauri-cdp-cloud-repair-chain.mjs`（本分支新构建） | 未跑（称 CDP 不可用） | **13/13 通过** ✅ 阅读无回退 |
| CDP 冒烟 `tauri-cdp-smoke.mjs`（本分支新构建） | 称失败 | **5/5 通过** ✅ |

符合需求、无需返工的部分：T1 经正式编辑事务（`EditOrigin::ListeningAudio`）写 part media 并尊重 `human_protected_targets`、
Q17–20 走 `unordered_set`；T2 所有生产编译点经单一分派；T3 学生端逐 part 音频、主检出 `server/dist` 未动、阅读零回归、
Electron E2E 通过；T4 模型给的 media 被丢弃并记 warning（有测试）；T5-2 发布后清理与永久删除是两条不同函数，发布不会删音频；
T5-4 旧钩子只剩注释。

### 1.2 必须返工的问题（按严重度）

**F1【高】学生端打不开的题被记成干净发布（T5-1）**
- 门禁结论为 Ready、但包检查失败而降级为 authoring-only 的条目，状态写成 `published`（`nas_package_v2.rs` `ItemPublication::item_status` 只看 `forced`，不看 `student_loadable`）。
- 对应测试 `forced_publish_degrades_one_unpackageable_item_and_publishes_the_rest` 断言 `bad_status == "published" || bad_status == "published_forced"`——**两边都能过的断言**，正是交接文件里的"恒过的场景"。
- 后果：题库显示"已发布"但学生端打不开；主验收链以 `status === "published"` 判干净通过，会**假绿**。

**F2【高】听力卷会出现在学生端「阅读」练习目录里（T3 交付物 3 未做）**
- 学生端 `server/src/lib/library/reading/NasJsDirectReadingAssetProvider.ts` `listAssets()` 的过滤是
  `entry.schemaVersion !== READING_V2_SCHEMA_VERSION || isRuntimeVersionCompatible(...)`——**非阅读条目一律放行**。
  T3 分支没有改这个文件。听力卷一发布就会进阅读目录，从那里点开会失败。

**F3【中】发布后解析缓存仍留在磁盘（T5-3 只做了一半）**
- 匹配改成精确身份是对的，但**发布后的清理 `library/final_version.rs::purge_source_artifacts` 根本没调用**
  `cleanup_parser_cache_for_job`。发布后 `<appData>/cache/parser/` 里这道题的解析产物（原文抽取结果）仍在，
  违反"发布后只保留可编辑最终版"。

**F4【中】T5-5 的 CDP 脚本在真实 App 里失败——脚本缺陷，产品正常**
- 质量方实跑 `tauri-cdp-single-file-import.mjs`：`pick-files-brings-exactly-the-chosen-file` 超时，后续级联失败。
- 已定位根因：抽屉打开后，「未连接云端，仅本地识别 · 去连接」一行在 profile 列表异步返回后才插入，
  把「选择文件」按钮**下推 49px**（y 138 → 187）。脚本在位移前量坐标、按坐标点击，点空了。
  直接调用 `automation_source_files` 返回恰好那一份；布局稳定后再点，恰好选中 1 份。
- 附带的小产品问题：真实用户手快也可能点错（布局跳动）。

**F5【报告失实，需更正】"本机 WebView2 CDP 已退化"不成立**
- 同一台机器、同一分支的新构建上，冒烟 5/5、阅读链 13/13 都通过。你那边的失败来自**你的执行环境**
  （进程环境里 `PATH` 首项被写坏、`HTTP_PROXY`/`HTTPS_PROXY`、`__COMPAT_LAYER=Installer` 等），不是机器。
- `findings.md` 的 `F-WEBVIEW2-CDP-UNAVAILABLE-2026-09-22` 记录了错误结论，必须更正；T5-5、T6 **不是**被机器阻塞。

**F6【报告措辞需更正】T5-2 的"用户能感知"不成立**
- `deleteJob` 在前端**没有任何调用方**；界面只有回收站（软删除）。实现本身符合任务书（挂在已存在的后端永久删除命令上、没有发明 UI），
  但"永久删除条目时受管音频跟着走"目前没有用户能触发。回报里改为"后端永久删除命令已覆盖音频清理，当前无 UI 入口"。
- 你顺带发现的 `delete_exam_by_id` 只删旧 `exams` 表、`library_items_v2` 留孤儿，登记为已知问题即可（同样无 UI 入口）。

---

## 2. 关于 CDP 运行环境（先解决它，否则 F4/T6 仍会卡住）

你的执行环境里 WebView2 起不来调试端点，而质量方环境里正常。二选一：

1. 在你的环境里启动 App 时使用**干净的进程环境**（去掉被写坏的 `PATH` 首项、代理变量、`__COMPAT_LAYER`），
   并用 `curl --noproxy '*'` 验证端点——你已经证明带代理的 curl 会给出假阳性；或
2. 如果环境无法修复：CDP 类验收（F4 复跑、T6）写好脚本、单元与命令处理器层证据做完后，**明确交给质量方实跑**，
   回报里标注"产品端到端待质量方执行"，不要写成"机器不可用"。

---

## 3. 返工任务

每条都先写会失败的测试、亲眼看红，再修。频繁提交，不要 push，仍在 `wave3-listening` 上。

### R1（F1）状态必须反映"学生端能否打开"
- 规则：`student_loadable == false` 的条目**永远不是** `published`。沿用 `published_forced`，或新增一个明确状态
  （例如 `published_not_loadable`）——二选一并在全链路（Rust 状态、`libraryTypes.ts` 映射与库行展示、`publish_records_v2`、
  `data-publish-outcome`）一致。库行必须能让用户看出"学生端打不开"。
- 把那条"恒过"断言改成**精确断言**；新增一条：门禁 Ready + 包检查失败 → 状态不是 `published`。
- 确认 `tauri-cdp-cloud-repair-chain.mjs` 的干净判据（`status === "published"`）在新状态下仍然正确。

### R2（F2）学生端阅读目录排除听力（另一仓库，沿用你的 worktree `F:\workspace\IELTS-NASfor-WenDao-listening`）
- `listAssets()` 只返回阅读条目（按 `modality`/`schemaVersion` 判定），对未知 schema 默认**不放进阅读目录**。
- 测试：混合清单（阅读 + 听力）→ 阅读目录只含阅读；纯阅读清单输出与改前逐字一致。
- 前后各跑一遍学生端全部测试（`run-all.cjs`、静态套件、Electron E2E），给出前后数字。重建的只能是你 worktree 里的 `server/dist`。

### R3（F3）发布后清理解析缓存
- 在 `purge_source_artifacts`（或其调用处）接上精确身份的 `cleanup_parser_cache_for_job`；失败只报告不回滚发布（与现有清理一致）。
- 测试：发布 → 该题的解析缓存消失；另一道 id 相近的题的缓存仍在。
- 说明：身份含源文件 sha256，两道题导入同一份 PDF 时会共享缓存条目——缓存可再生，允许被一并清掉，但在回报里写明。

### R4（F4）修 `tauri-cdp-single-file-import.mjs` 并拿到产品端到端证据
- 点击前等布局稳定：例如等「未连接云端」这一行出现/云端状态已决，或连续两次读到的按钮坐标一致再点。**不要**改成 `element.click()` 绕过真实点击。
- 产品小修（可选但建议）：给那一行预留高度或同步渲染占位，避免按钮在用户眼前跳动；如做，加一条前端测试。
- 按第 2 节解决环境后实跑到通过；做不到则交质量方实跑。

### R5（F5/F6）更正记录
- 更正 `findings.md` 的 `F-WEBVIEW2-CDP-UNAVAILABLE-2026-09-22`：结论改为"执行环境问题，非机器"，保留排查过程作为环境踩坑记录。
- 更正 `progress.md` 中 T5 的第 2 条与 T5-5 证据等级的表述（见 F6、F4）。

### R6 T6 听力真实 App 验收（不再视为被阻塞）
按原任务书 T6 执行：导入 `listening-vol7-t9.pdf` → 检测弹窗 → 绑定 4 段音频（无真实 MP3 时用生成 WAV 并标注）→
草稿 4 parts / 40 slot → 各 part 播放器加载受管音频 → 发布 → 学生端（用 T3/R2 的 worktree 构建）真实 provider 加载并取到每个 part 的音频。
环境仍不行则写好脚本交质量方实跑。

---

## 4. 质量方收货标准

- 重跑三套测试数字与回报一致；阅读链保持 13/13。
- R1：库里不存在 `student_loadable=0` 且状态为 `published` 的条目（质量方会直接查 `publish_records_v2` 与 `library_items_v2`）。
- R2：学生端混合清单测试 + 前后数字；主检出 `server/dist` 未变。
- R3：发布后 `<appData>/cache/parser/` 不含该题条目。
- R4：`tauri-cdp-single-file-import.mjs` 真实 App 通过（6/6）。
- 抽查先红：回退实现提交，对应测试应失败。
