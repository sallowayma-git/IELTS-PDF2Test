# IELTS Document Recognition Phase 3 C-001 完成记录

本记录对应总计划 `Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md` 的
16.4「DOCX 富结构层」首个 P0 任务 C-001「安全 package/relationship reader」。
本任务为 Phase 3 的 package 基础设施，不提前实现 style、numbering、table、media
或 render fallback 语义层。

## 已完成

- 新增 `src-tauri/src/docx_ingest/package.rs` 与 `mod.rs`，以只读方式打开 DOCX
  ZIP/OOXML package。
- 入口检查 `.docx` 扩展名、`PK` magic 和物理 archive 大小；默认限制为：
  - archive 128 MiB；
  - 最多 4096 个条目；
  - 总解压大小 256 MiB；
  - 单条解压大小 64 MiB；
  - XML part 16 MiB；
  - part path 1024 bytes；
  - relationship 8192 条。
- 拒绝 zip-slip、绝对路径、Windows 分隔符/驱动器路径、父目录/当前目录组件、
  控制字符、跨平台不安全字符、大小写碰撞路径、符号链接、加密条目、重复条目和
  超限条目；不执行 ZIP 内的任何文件。
- 解析 `[Content_Types].xml`，保留 Default/Override 映射并校验 Override 指向
  的 part 存在。
- 解析 `_rels/.rels` 和各 part 的 relationship part；内部 target 做相对路径解析、
  根目录越界检查和包内存在性检查；`TargetMode=External` 只保留描述，不解析、不
  访问网络。
- 主文档优先由根 relationship 的 internal `officeDocument` target 选定；为兼容
  仓库现有最小 synthetic fixture，根 relationship part 可以缺省，此时回退到
  `word/document.xml`，且选中的主文档 part 必须存在。
- 现有 Rust DOCX V1 parser 改用安全 package reader 取得主文档 `document.xml`、
  `styles.xml`、`numbering.xml`；`DocumentIRV1` block 形状、V1 输出契约和
  Python fallback 的正常语义不改变。被 package 安全策略拒绝的输入直接生成
  failure IR，不再把同一恶意 ZIP 交给 legacy fallback。

## 验收覆盖

| 验收点 | 覆盖方式 |
| --- | --- |
| valid package / content types | synthetic package 读取、Override/Default 查询 |
| root/part relationships | main document、relative `..` target、source part 映射 |
| external relationship | 保留关系但 `resolvedTarget=None`，不触网 |
| zip-slip/path safety | `..`、绝对路径、Windows drive/separator、大小写碰撞 |
| symlink/encryption boundary | symlink entry rejection；encrypted entry policy |
| zip bomb limits | entry count、单条解压大小、总解压大小、archive/XML 上限 |
| malformed package | 缺 content types、缺 document、坏 XML、坏 relationship target |
| V1 compatibility | complex DOCX、table、style/numbering、section columns、merge regression |

## 验收命令

```text
npm run verify:phase0:strict
npm run check
npm run build
npm run verify:phase1:schema
npm run verify:phase2:shadow
npm run verify:phase3:docx-package
cargo test --manifest-path src-tauri/Cargo.toml docx_ingest -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml docx_ooxml -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml complex_docx_fixture_reaches_authoring_ir -- --nocapture
```

全量 Rust 测试仍可能包含 Phase 0 已记录的既有环境失败项；C-001 新增测试与
DOCX 回归不得失败。

## 运行边界与下一步

- 没有新增或开启生产 V2 flag；`documentIrV2`、`authoringV2`、
  `runtimeSourceV2`、`nasPackageV2`、`listeningV1`、`documentIrV2Shadow` 和
  `pdfPerQuestionLlmRepair` 继续默认关闭。
- 本任务不写 V2 shadow artifact、不进入 authoring/export、不修改 PDF Phase 2
  产物；V1 仍是当前 authoritative path。
- 下一项是 C-002：在该 package reader 之上增量解析 style cascade 与 numbering
  definitions，仍应先以 V2/旁路证据形式接入，不覆盖 V1。
