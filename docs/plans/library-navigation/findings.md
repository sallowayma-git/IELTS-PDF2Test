# 发现

LibraryPage 当前未消费 router 已解析的 modality；Reading/Listening 混合表格后总是挂 WritingPanel。
LibraryPaperTable 总会创建 P1/P2/P3 和听力组，即使筛选后组内为空，需避免空组误导。
组卷状态在 LibraryPage，切换类别不应重置；WritingPanel 的未保存内容在组件状态中，隐藏视图时应保留组件。
真实 Tauri dev 运行中：Vite 1420，app PID38651，隔离数据 /tmp/pdf2test-devtest-data，主源码通过 HMR 更新。
原生 CUA app bridge 在本机启动失败，若无法恢复，必须将浏览器交互验证与真实 Tauri UI 验证区分。
