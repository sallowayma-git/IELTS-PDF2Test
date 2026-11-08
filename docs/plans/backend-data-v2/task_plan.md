# 后端数据结构 V2 实施计划

目标：按用户完整任务书完成作者端与学生端数据层、兼容迁移、真实采集、档案导入、判卷审计命令与验收。保留 main 未提交内容；独立 worktree；不 push；按里程碑提交并添加指定 Co-Authored-By。

- [ ] M1 核实现状、先定契约、两端迁移及 V1/V2 兼容
- [ ] M2 学生端 slot、事件、听力、元数据与快照 V2，服务与 Electron 验证
- [ ] M3 作者端发布档案、内容冻结、历史恢复与真实快照导入
- [ ] M4 详情、改判、重判、写作版本、审计、统计与导出命令
- [ ] 最终检查、跨仓库产品链路、报告与分组提交

工作目录：
- 作者端 `/Users/maziheng/Downloads/Desktop/copy/PDF2Test-backend-data-v2`
- 学生端 `/Users/maziheng/Downloads/Desktop/IELTS-NASfor-WenDao-backend-data-v2`

## 约束
不修改原始作答；只读 ready 快照，不打开 NAS 终端活库；不存学生本机作答；统计带 N；不显示 IELTS Band；前端页面不在范围。

## 错误与限制
- 现行跨仓库 Tauri E2E 写死 Windows .exe / WebView2；本机 macOS，需诚实标注产品验证缺口。
