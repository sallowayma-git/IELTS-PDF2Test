运行时：reading 0.2.0 / listening 1.0.0，不与安装包 0.0.2-beta 比较。听力 entry 与 source.audit 两道门禁都用同一比较器。
新增真实缺陷：音频读取遗漏 resourcesBase；作者发布端版本校验写死 0.2.0。两者必须修复才可发布并加载真实听力批次。
逆补丁：TS patchExpression 共用数组造成 restoreStructure 两路径互相影响；分别 clone 后消除。
独立复核修正：听力 Part 声明只含 scoring；新增组继承时删除模板缺失策略与 optionBankRef。
V1 诊断提交 f9226902eedb4872f33e88a4adfe9508b3e1f54a：aligned_short_line 合并短行尾。几何排版的旧/新数量：chili55/49,conformity58/50,fish48/41,ocean60/53,organisational138/138,petri67/57,sleep58/49,western104/102。最终产物可能添加空页占位和图片块，待独立实际解析统计。
