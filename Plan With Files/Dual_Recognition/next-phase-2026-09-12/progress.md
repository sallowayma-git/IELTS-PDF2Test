# Progress

## 2026-09-12

- 读取 `planning-with-files` 技能说明并运行 session catchup；未发现需要恢复的未同步会话。
- 枚举仓库、审计目录、专项规划目录及最近提交。
- 确认工作树干净，当前 HEAD 为 `3dfd6c0`。
- 创建本次独立规划作用域，开始阶段 1。
- 完整读取审计入口、审计任务计划、根任务计划与审计过程记录；读取汇总报告/发现登记的高优先级部分。
- 初步确认下一阶段不能直接照搬冻结基线上的 157 项发现，必须先对当前 HEAD 做差异化复盘。
- 完成三路只读核验：当前高优先级缺陷、产品验收门、后续里程碑依赖。
- 关键结论：当前 HEAD 无新鲜完整 Tauri 成功报告；数据正确性与 `user_edited` 护栏仍开放；M4 仅部分接入；F3/F8 中返回按钮存在 selector cascade 风险。
- 抽查关键 `file:line`：返回按钮 CSS 级联、processing 导入/取消路径、M4 quality 消费、CI 门与旧 Tauri 失败报告，探子结论成立。
- 产出 `NEXT_PHASE_PLAN.md`，定义 G-1、G0–G7、两个迭代切片、agent 边界与 D0–D7 决策门。
- 本次只写规划文件，未修改或验证产品代码；规划任务完成。

## 2026-09-12（接手会话：D0/G0/G1 执行）

- 从上一会话日志恢复现场：D0 已提交（5548f7a），G1 只有工作流设计稿与一处残留未提交改动（A4-F01 半成品），G0 未执行。
- 完成 G1 全部修复并提交 761afd9：A4-F01 补偿删除+启动孤儿清理、A4-F02 durable 取消（schema v3 cancel_requested_at）+ post-local/post-cloud 取消检查 + advance 原子强制取消、A4-F03 retry_exhausted（含 v3 存量数据修正）+ 前端诚实文案、A4-F05 reclaim 成功守卫、P0-1 终态清 lease、P0-2 取消跨重启兑现、A7-F02 迟到收尾/候选不覆盖 canonical 回归测试。
- 两路独立只读对抗复审（PASS-WITH-FINDINGS）：3 项 P1 全部修复（取消 TOCTOU、补偿删除误删既有文件、lease 丢失免 lease 终态收尾），P2 修复 4 项、其余记录在案。
- cargo test 591 通过；npm run check / test / build 通过。
- G0：重建 debug exe（staleBuild=false），四条真实 Tauri 链全部运行：workspace-edit passed、workspace-back passed（D0 computed style/flush 断言全过）、完整链 passed-with-publish-gate-blocked、publish blocked（质量门正确拦截，仓库 fixture 达不到 ready，私有语料本机缺失 → UI 级 publish 通过记 CANNOT-RUN，机制由命令级 product_chain 覆盖）。
- E2E 过程中发现并修复真实产品缺陷：set_item_status_ready 状态提交晚于最后事件且补发事件被 stateVersion 去重丢弃 → 题库行永久显示"排队中"（提交 event_seq 递增 + 补发事件，aaa7df3）。
- E2E harness 三处修复：stale 点击重试、publish 目标目录语义对齐（nas-library）、完整链缺前置导航与门禁文案识别。
- G0-T06：windows-smoke.yml 新增 tauri-e2e-smoke job（CANNOT-RUN 语义显式失败，不假绿；CI 运行待验证）。
- 报告：artifacts/e2e-tauri/G0-BASELINE-2026-09-12.json。

## 2026-09-12（第二轮：验收缺口收敛）

- 门状态修正：D0 部分通过 / D1 部分阻塞 / D2 部分通过（不再以总结宣布三门关闭）。
- D0：withBusy 去 re-throw（5548f7a 的错误归因修正）；返回按钮测试重写为 12 步（DB 写锁真实阻塞保存证明 flush 等待、marker 重开持久、busy 超时真实失败不导航+错误可见+0 unhandledrejection、Tab focus-visible、双击单批保存）；变异验证：移除 flush 后三步失败、恢复后全过。
- freshness 扩展到 src-tauri/源码+构建配置+锁文件；陈旧构建从告警改为 CANNOT-RUN 硬失败；报告钉 HEAD/工作树/exe SHA256/fixture SHA256（buildIdentity）。
- G1 边界：advance_stage 返回有效阶段（等待 cloud permit 期间取消不再启动云调用）；finalize_cancelled_without_lease（lease 过期后取消不必等重启）；staging 失败补偿 job 壳；migration 强化回归（真实 revision + journal 幂等重放断言）。594 Rust 测试绿。
- 取消/重启恢复真实 Tauri E2E 全过（批量导入→UI 取消→强杀重启→取消持久→恢复达稳定状态）。
- 发布路径收敛：publish-ready E2E 全过——仓库 proven-ready fixture 预置 + 真实 UI 发布按钮 → 质量门重算 → NAS 包落盘。明确 scope：只验证发布链，不代表自动识别链达 ready（后者保持 blocked）。
- 提交：28706df（D0/G1 主体）、d98380f（测试脚本修正）。
- 报告：artifacts/e2e-tauri/G0-BASELINE-2026-09-12-v2.json。

## 2026-09-12（第二轮收尾：复核与基线对齐）

- 两路独立只读复核均为 PASS-WITH-FINDINGS、无 P0；发现全部处置（修复 8 项、记录 5 项）。
- 复核 A 的 P1（基线 run 与报告 exe SHA 不一致）已修复：全部脚本报告钉 headSha+exeSha256，v3 基线与六个 run 逐项交叉核对一致（exe 4c51e067…，HEAD d98380f）。
- 变异验证：移除返回按钮 flush → 三步红；恢复 → 全绿。D0 测试无假绿。
- 提交链：761afd9 → aaa7df3 → 28706df → d98380f → 91828ea。
- 最终报告：artifacts/e2e-tauri/G0-BASELINE-2026-09-12-v3.json（六链、门状态、三层保护分项、遗留清单）。
- 门状态维持：D0 部分通过 / D1 部分阻塞 / D2 部分通过。

## 2026-09-13（G2 启动轮）

- 阶段1：FINAL-HEAD-ACCEPTANCE-91828ea.json（write-tree=HEAD tree=f667fc28…，exe 2ddb236e…，四链 12/12、6/6、8/8、4/4 全过）→ D0 升级为 PASS；v3 报告不再作为最终 HEAD 证据。
- 阶段2：Phase 4 指标 runner（b0c868f，report-only）——21/39 fixture 实测（16 私有缺失、2 SHA 漂移如实排除），聚合：empty prompt 0、visual fallback 3/3、matching 1/2、statement 3/3、recall=no_reference（参照在 private-real metadata）。产物含 runToken/HEAD/write-tree/manifest SHA/逐题结果。
- 阶段3：direct canonical（a9aa735，flag QLG_DIRECT_CANONICAL 默认关）——QLG+物理层+pre-V1 答案候选直出 IeltsAuthoringIRV2，不读 V1 authoring；3 个契约测试 + 差异测试（逐 fixture 对比报告）。
- 阶段4：verifier 八向量攻击 PASS-WITH-FINDINGS（无 P0）：向量 1/2/5/6/7/8 捕获或大体捕获，3/4 部分可穿透（测试覆盖面）。P1-a（passage 分段丢尾行）+ P2-a（TFNG 语句选项空 content）+ P2-e 已修复（091db16，601 Rust 绿，含尾块复审测试）；P2-b/c/d/f 记录待下片。
- 边界：A3-F04 单独记录未混入；finalize 瞬时 DB 错误维持 P2；M5 未动；publish-ready 仅证明发布机制。
- G2 完成判定未达成：需指标达阈值（依赖 private-real 语料）、连续两轮无未裁定 P0/P1、真实 Tauri 回归。

## 2026-09-13（G2 语义修复 + 双轮对抗）

- 门状态新口径：D0=91828ea 时点 PASS（当前 HEAD 回归已重跑见下）；D1=BLOCKED（发布机制已有部分产品证据）；D2=部分通过；G2 metrics=runner 已建、指标定义与被测链 v2 重写完成；G2 direct canonical=flag-off prototype；M5=未开始。
- 组1（b40f918）：TFNG/YNNG 分开、MC cardinality 解析（Choose TWO→exact=2，未解析→MULTIPLE_CHOICE_CARDINALITY_UNRESOLVED）、缺块保留题号+QUESTION_BLOCK_MISSING、visual 物化/阻断、真实 sourceFileId+哈希交叉校验+ANCHOR_TARGET_INVALID 构建期校验、option id scope 唯一、未分配按页聚合。正反测试 9 个。
- 组2（54e7810）：metrics v2——expected 分母、direct 被测链、legacy side-by-side、MIN_SAMPLE=5/insufficient_evidence、corpus 分类、clean 树门、indexTreeIdentity、supersedes。正式报告：artifacts/phase4-metrics/report-54ea5fcc….json（synthetic 基线：direct empty prompt 55/57、visual 5/5 失败、statement 0 覆盖——如实）。
- 组3：轮1 攻击（无 P0，P1×3：panic/锚点归属/指标跨桶）全部修复（f085933）；轮2 verifier 六项检查 PASS-WITH-FINDINGS（P0=0 P1=0，2×P2 已修 f119b2a，5×P3 记录）。
- 组4：exe 重建（54e7810a 代码）、flag-off workspace-edit passed（13-09-53 run）；flag-on PDF/DOCX Tauri E2E 与第三轮确认 pass 待下片。
- 完成口径：synthetic=开发证据；private-real 缺失不得宣布达标；flag 默认关；未进入默认启用。

## 2026-09-13（G2 flag-on 产品验证续行）

- 已核对未提交 E2E 支撑改动：新增 `PDF2TEST_AUTOMATION_SOURCE_FILES` 的真实 Tauri 文件选择钩子、Tauri command、前端原生对话框回退，以及 harness `appEnv` 透传；普通用户路径在环境变量缺失时保持原生文件对话框。
- 修复共享 harness `writeReport` 格式破损；为题组根节点增加 `data-editor-id`，使 task-level actionable issue 能定位到真实 DOM。
- 定向验证通过：`npm run check`；`cargo test --manifest-path src-tauri/Cargo.toml direct_canonical -- --nocapture`（9/9）。
- 当前进行中：编写 `QLG_DIRECT_CANONICAL=1` + `LOCAL_RECOGNITION_BLOCKERS_GATE=1` 的 PDF/DOCX 双样本真实 Tauri E2E；必须检查 direct audit note，发现 V1 fallback 即失败，不以可打开工作区代替 direct 链证明。
