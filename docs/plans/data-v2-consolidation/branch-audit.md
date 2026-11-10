# 分支收敛与清理

| 仓库 | 分支 | 结论 |
|---|---|---|
| 作者 | codex/backend-data-v2 | 选为主实现，已随PR#6合入main，远端分支删除 |
| 作者 | codex/library-navigation | 已纳入PR#6；原PR#5自动标为merged，远端分支删除 |
| 作者 | codex/data-v2-consolidation | PR#6合入后删除远端分支 |
| 作者 | codex/backend-data-v2-macos-20261109 | 必要修复选择性吸收，其余舍弃；tag归档后删除远端分支，不称为完整合入 |
| 学生 | codex/backend-data-v2 | 选为主实现，已随PR#5合入main，远端分支删除 |
| 学生 | codex/data-v2-consolidation | PR#5合入后删除远端分支 |
| 学生 | types-round | 原已是main祖先，删除远端残留分支 |
| 学生 | codex/backend-data-v2-macos-20261109 | 必要修复选择性吸收；tag归档后删除远端分支 |

两仓均使用带说明的归档标签archive/data-v2-macos-20261109：作者指向699c689，学生指向595631d。清理前分别核对当前远端tip、main祖先关系或tag目标，保留完整追溯。验证记录的收尾分支会在合入后删除，最终远端仅保留main。
