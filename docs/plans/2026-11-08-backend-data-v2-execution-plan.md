# 后端数据模型 V2：实施准备与调度清单

状态：准备阶段；M1–M4 尚未实施。依据同目录 `2026-11-08-backend-data-model-prompt.md`。本文件不替代生效契约。

## 工作区与职责

| 项目 | 作者端 | 学生端 |
| --- | --- | --- |
| worktree | `F:/workspace/PDF2Test-backend-data-v2` | `F:/workspace/IELTS-NASfor-WenDao-backend-data-v2` |
| 分支 | `codex/backend-data-v2` | `codex/backend-data-v2` |
| 起始 HEAD | `a7fa8ca` | `f685552` |

不在 main 修改，不 push。按准备、M1、M2、M3、M4 分组提交，结尾添加任务书要求的 Co-Authored-By。原工作区未跟踪文件不清理、不覆盖。

主线程：亲读基础文档和即将修改的确切代码；决定数据模型/迁移/错误语义；实现所有源码修改；集成与最终验证。子代理：只读探索与独立核验，不写代码、不替产品决定口径。

模型：用户要求 luna max，当前角色配置不支持；用户先要求暂缓，随后允许 terra，并明确模型名 `gpt-6.1-terra`。该精确模型名已被 spawn_agent 接受；已按此参数派发 E01–E06。`terra` 简名曾被拒绝，不能使用它或静默回退 luna low。后续派发前再次以工具实际结果为准。

## 派发与回收协议

每张卡只解决一个有限问题，通常 1–3 文件；必要的直接调用点可追加，跨范围搜索须说明原因。禁止访问真实学生作答 NAS、安装依赖、提交或派生子代理。大文件先按符号定位再读区间；不把架构/设计/交接文档的阅读外包。

统一返回：任务 ID；最多 750 中文字；事实/推断/未核实项；每个关键结论附 `file:line`、符号名与 1–2 行原文；最多 3 个主线程复核点。子代理结论先记录为线索，只按出处点验重要部分。独立任务同批并发，单批最多 6 个；完成即回收名额。

可复制任务模板：

```text
任务 <ID>；模型 gpt-6.1-terra；角色只读探子。
仓库/基线：<worktree>/<HEAD>。
范围：<表内文件或符号>；禁止修改与真实 NAS 访问。
问题：<单一目标和具体核对项>。
输出：事实/推断/未核实 + file:line/符号/关键原文，最多750字。
停止条件：回答问题即返回；不能凭旧测试报告声称当前产品通过。
```

## 探索任务卡

所有未派发任务须等直接依赖完成后，按实际修改差异进一步收窄范围。这里列出的范围是上限，不是全目录通读要求。

| ID | 范围 | 唯一目标 / 需要返回的证据 | 依赖 / 状态 |
| --- | --- | --- | --- |
| E01 | 学生 MachineSubmissionStore、SubmissionSnapshotService、machine-store-connection 测试 | 迁移挂点、版本键所有者、旧库测试入口 | 已回收，关键出处已点验 |
| E02 | 作者 analytics/import.rs、analytics/tests.rs | 重导入删除/覆盖边界及教师数据保留风险、快照夹具来源 | 已回收，关键出处已点验 |
| E03 | 作者 nas_package_v2 的 publish_items_core/指针直接调用、library/schema 迁移接口 | runtime 原文可取位置、指针提交点、本地档案挂点 | 已回收，关键出处已点验 |
| E04 | 学生 ExamListeningService、ExamFinalSubmitService、ExamSuiteService；provider 单个加载函数 | 听力最终提交/事务关系、锁定证据、环境变量/版本门禁读点 | 已回收，关键出处已点验 |
| E05 | 两端维护中的双产品链脚本、构建入口、学生 snapshot 测试 | 真实 E2E 启动条件、听写覆盖、真实快照夹具 API | 已回收，关键出处已点验 |
| E06 | reading useReadingAttempt、ExamReadingService、store 答案映射函数 | question_id→slot、矩阵/多选行映射、现有保存入口 | 已回收，关键出处已点验 |
| E07 | 学生 runtime 配置类型与解析的直接函数 | session_id 如何从配置到签到落库，不接 UI | E01 后待派发 |
| E08 | 学生 ExamAttemptService 的 abandon 分支及信标函数 | 作废原因与终端所有权校验挂点 | E01 后待派发 |
| E09 | 学生阅读页切 Part 的处理器和当前 autosave 调用 | part_enter/leave 的准确触发点、切回与卸载时事件丢失边界 | E06 后待派发 |
| E10 | 学生 reading/listening 输入与焦点处理器 | answer_changed/focus 事件是否可复用当前保存载荷，不记逐字键盘 | E06 后待派发 |
| E11 | 学生 final-submit 的 retry/rollback/idempotent 三个测试 | 听力事务接入后须保留的回执语义与故障断言 | E04 后待派发 |
| E12 | 作者 analytics/queries 的 load_attempts/validity/exam_view | counted 与有效分查询需要统一的最小入口 | E02 后待派发 |
| E13 | 作者 analytics/queries 的 question_view 与矩阵构造 | slot 复合键、不同卷同题号隔离及 N 分母 | E06、E12 后待派发 |
| E14 | 作者当前 settings 存取类型及 Tauri 注册区 | 操作人姓名的现有配置入口与新命令注册位置 | 待派发 |
| E15 | 作者 analytics/export.rs 的 workbook 总入口 | 三张新增表共享查询挂点、导出成功审计位置 | E12 后待派发 |
| E16 | 作者真实发布产物 source 文件名与编译结果对象 | reading/listening 内容上下文提取边界、hash 所覆盖的字节 | E03 后待派发 |
| V01 | 主线程完成后的学生迁移差异与新增迁移测试 | 独立核验旧数据/重复启动/回滚/版本标记一致 | M1 实现后 |
| V02 | 主线程完成后的作者导入差异与学生生成夹具 | 独立核验 V1/V2 整库导入、同 attempt 重导入保留教师层 | M1/M3 实现后 |
| V03 | 主线程完成后的听力提交和锁卷差异 | 独立核验重发卷中途仍按 pin 判分、失败无半成品 | M2 实现后 |
| V04 | 改判/统计差异和针对性测试输出 | 独立核验部分分、撤销恢复、题型/矩阵/导出同口径 | M4 对应实现后 |
| V05 | 写作 CAS/批注/审核/发布差异 | 独立核验单一 published 版本、冲突无副作用、UTF-16 引文 | M4 对应实现后 |
| V06 | 最终报告 JSON 与命令日志，限定本轮产物 | 独立核验源码归因、workarounds=[]、证据层级不混淆 | 最终验收后 |

## 主线程实施工作包

下表均由主线程修改。每项完成后记录改动路径、迁移影响和实际验证结果；同一文件的工作包串行，不把设计未定稿的问题交给子代理自行选择。

| ID | 小工作包 | 前置 | 验收点 |
| --- | --- | --- | --- |
| P01 | 完整读取两份现行契约、AGENTS 和任务书 | 无 | 已完成 |
| P02 | worktree、基线与原现场登记 | P01 | 已完成；两端分支可解析 |
| P03 | 探索证据点验与现有/缺失矩阵 | E01–E06 | 已完成首批；未核实独立标注，见 preparation-report |
| C01 | 快照 V2 兼容表与异常语义定稿 | P03 | V1/1 与 V2/2 均可消费，过新拒绝 |
| C02 | 学生 schema v2 DDL/回填/事件幂等定稿 | E01、E06 | slot 与原始分语义唯一，版本只在提交后推进 |
| C03 | 本地档案/分析/教师层归属和迁移编号定稿 | E02、E03 | 分库边界明确，无双库原子事务假设 |
| C04 | 新命令 DTO、CAS、错误码、N、审计边界定稿 | C03、E12–E15 | 不破坏现有 analytics_* 返回兼容性 |
| M1-S01 | 学生旧库版本探测与原子迁移骨架 | C01、C02 | 空库、无版本键旧库、V1、V2、过新库 |
| M1-S02 | reading_answers 三列与 score/max_score 回填 | M1-S01 | 未判分保留 null，不冒充 0 分 |
| M1-S03 | exam_attempts 三列与事件/听力表 DDL | M1-S01 | FK/唯一键/索引可重复启动 |
| M1-S04 | 迁移后版本读取与导出 meta 绑定 | M1-S02、S03 | 内部版本与 meta 一致，无提前标 V2 |
| M1-A01 | analytics 版本化迁移骨架与字段回填 | C03 | 已导入旧记录不丢失 |
| M1-A02 | 档案与内容快照迁移 | C03 | 发布事实和内容唯一键、可恢复状态 |
| M1-A03 | 教师层表/索引/审计 DDL | C03、C04 | 原始表不因教师操作更新 |
| M1-A04 | 导入 V1/V2 表结构分支与最小真实夹具 | S04、A01 | 缺可选表与伪造/过新版本正确处理 |
| M1-V01 | 升级/重复打开/中途失败回滚测试与报告 | 所有 M1 | 不能把 schema 通过写成产品通过 |
| M2-S01 | 阅读 slot 载荷→store 映射 | E06、M1 | 多选/矩阵每个得分点唯一 |
| M2-S02 | 服务端事件批量写入/去重 | C02、M1 | 重试不重复，事件失败不影响答案保存 |
| M2-S03 | 阅读改答历史内存队列接 autosave | E09、E10、S02 | 无 localStorage/IndexedDB/本地磁盘草稿 |
| M2-S04 | Part/应用/题目焦点事件接现有通道 | E09、E10、S02 | 回切顺序、事件断档显式标记 |
| M2-S05 | 听力统一 runtime 版本与仅认指针/既有 pin | E04、M1 | env 在 exam 模式不能决定 examId |
| M2-S06 | 听力汇总/逐 slot/播放快照事务落库 | E04、E11、S05 | 不更改原始行；提交重试与回执规则一致 |
| M2-S07 | session/student_uuid 预留与作废原因落库 | E07、E08、M1 | 跨终端访问仍拒绝；不做名册 UI |
| M2-S08 | 快照 V2 发布与 pending 重试 | S01–S07 | ready 最后；导出失败不使交卷失败 |
| M2-V01 | 服务层 + 真 Electron + 快照新表断言 | 所有 M2 | 静态套件先于 Electron E2E |
| M3-A01 | 指针提交前固化 runtime source 本地证据 | E03、E16、M1 | 保存 hash 对应原文，禁止读最新 canonical 替代 |
| M3-A02 | 指针成功后档案落库/旧卷 superseded | A01 | NAS 成功、本地失败有可重试恢复状态 |
| M3-A03 | 历史 JSON 幂等补建与内容缺失报告 | A02 | 资源已清理不得伪造内容 |
| M3-A04 | 写作正文/原文引用/高亮笔记计时导入 | M1、M2 | 来源路径与学生真实生成载荷一致 |
| M3-A05 | 事件/听力镜像导入及重导入保护 | A04、E02 | override/grade/audit/counted 不被刷新覆盖 |
| M3-A06 | 只读题目上下文与试卷列表命令 | A03、A04 | hash 不匹配与缺内容返回明确异常 |
| M3-V01 | 学生真实生成 V1/V2 快照的 Rust 回归 | 所有 M3 | 禁止仅手写学生端不会产出的 sqlite |
| M4-A01 | 操作人配置与结构化中文错误适配 | E14、C04 | 缺身份写入策略按定稿契约执行 |
| M4-A02 | create/revoke/list override | A01、M3 | 原始不动、范围/分值校验、审计同事务 |
| M4-A03 | 有效分/原始分共享查询层 | A02、E12、E13 | 分布、题型、难题、矩阵、N 一致 |
| M4-A04 | set_attempt_counted 与原因审计 | A03 | 不覆盖学生原始 status；恢复不掩盖无效数据 |
| M4-A05 | preview_rejudge 纯查询 | A03 | 返回影响范围与当前版本，无写入 |
| M4-A06 | apply_rejudge 与批量 override | A05 | preview 后状态变化重查冲突，整批事务 |
| M4-A07 | writing grade draft/CAS 与批注范围 | M3、A01 | 四维边界、引用匹配、冲突无写入 |
| M4-A08 | writing review/publish 状态流转 | A07 | 同作文最多一版 published、历史保留 |
| M4-A09 | list_writing_queue 与 list_audit_log | A07、A08 | 过滤/分页/N，不泄漏跨对象写操作 |
| M4-A10 | get_attempt_detail 聚合 | M3、A02–A09 | 原文/题干/改答/写作全文/听力与异常齐全 |
| M4-A11 | xlsx 三张新增表与导出审计 | A03、A08、E15 | 原始/有效分区分；声明和 N |
| M4-A12 | Tauri 注册和命令边界回归 | A01–A11 | 每命令成功、冲突、不属于目标路径 |
| M4-V01 | 改判撤销/写作发布/重导入组合验证 | 所有 M4 | 分层事实不可变，统计与审计可追溯 |
| F01 | 两仓库完整回归 | M1–M4 | 下述真实命令逐条记录 |
| F02 | 双产品全链听力/写作/教师命令验收 | F01 | workarounds=[]，发布中途锁卷保护仍过 |
| F03 | 独立核验、迁移说明与最终报告 | F02、V06 | 未完成、私有用例跳过与产品问题单列 |

## 验证矩阵与执行约束

准备阶段只检查文档、路径、引用、工作区和提交；不宣称迁移或产品功能通过。实现后再执行下列命令，保存完整退出码和报告路径。

| 层级 | 位置与命令 | 额外断言 |
| --- | --- | --- |
| 单测/类型 | 作者：`npx tsc --noEmit`；`npx vitest run`；`cargo test --manifest-path src-tauri/Cargo.toml` | 私有语料忽略逐条列出，不能报成全覆盖 |
| 服务/命令 | 学生：`npm run build:server`；`node developer/tests/exam/run-all.cjs`；在 apps/student-exam 运行 `npx vitest run`；`node --test developer/tests/js/*.test.js` | Windows 的 glob 由实际工具支持验证，必要时显式枚举文件参数 |
| 静态契约 | 学生 PowerShell：`$env:IELTS_PDF2TEST_REPO='F:/workspace/PDF2Test-backend-data-v2'` 后 `py developer/tests/ci/run_static_suite.py` | 仅 schema/静态证据；任何 skip 明列 |
| 产品 Electron | 静态套件之后：`py developer/tests/e2e/suite_practice_flow.py` | 验证当前构建、NAS 上快照新表与真实填写值 |
| 产品双端 | 作者：配置 `STUDENT_REPO` 为学生实施 worktree，执行 `node scripts/e2e/cross-repo-full-chain.mjs` | Tauri 发布→Electron 听/读/写→Tauri 导入→detail→改判→写作发布→统计/导出/审计 |

共享 node_modules 之前先检查真实路径/链接属性；学生根使用目录链接时禁止根 npm install/ci。`apps/student-exam` 必须是真实私有目录，其构建可能执行 npm ci。不得以复制旧 dist/exe 通过当前源码验收。双产品链的诊断启动参数与 workarounds 数组分别报告，不混为一谈。

## 产品待确认（不阻塞准备，不擅自改变规则）

1. 任务书“不显示 IELTS Band”与写作四维 0–9、task_band/写作成绩表的对外展示边界。准备稿保留需求，不推导官方等级、不自行确定显示口径。
2. 无账号体系时“权限外路径”的具体含义：本轮可验证终端归属、对象归属、状态与 CAS；是否还要求某种操作者授权需要产品确认，不能用姓名字符串冒充认证。
3. 原始→复核→发布成绩需要独立发布事实；任务书已有写作 published 版本，但阅读/听力成绩发布实体与冻结时点尚未给出，实施前明确。
4. 作废后是否允许仅通过 counted 恢复统计，或需独立作者端作废/恢复业务状态；无效原始成绩不得因 counted=true 被自动视为可判分。

## 每里程碑报告模板

中文记录：提交/改动文件；实际命令与退出码；旧库升级/回填/回滚；产品 E2E / 服务与命令 / 单测与 schema 三层证据；未完成及不确定项；产品待确认项。准备稿和历史通过记录不能计入当前验收。
