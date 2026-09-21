import { describe, expect, it } from "vitest";
import { toUserFacingError, userMessageOf } from "./userFacingError";

// 证据层级：pure unit（计划 §19.1 层 1）。断言「机器码 -> 人话」的稳定映射，
// 一旦分类回归（例如又把原始机器码直送 UI）这些用例会失败。

describe("toUserFacingError — 已知机器码分类", () => {
  it("把 EDIT_VERSION_CONFLICT 归为 conflict 并给出刷新提示", () => {
    const result = toUserFacingError(new Error("EDIT_VERSION_CONFLICT:current=2:base=1"));
    expect(result.category).toBe("conflict");
    expect(result.userMessage).toContain("刷新");
    expect(result.internalDetail).toBe("EDIT_VERSION_CONFLICT:current=2:base=1");
    expect(result.code).toBe("EDIT_VERSION_CONFLICT");
  });

  it("把 ITEM_DS_NOT_SEEDED / AUTHORING_V2_NOT_AVAILABLE 归为 not_ready", () => {
    expect(toUserFacingError("ITEM_DS_NOT_SEEDED:import-1").category).toBe("not_ready");
    expect(toUserFacingError("AUTHORING_V2_NOT_AVAILABLE:shadow_missing").category).toBe("not_ready");
  });

  it("把 ITEM_NOT_FOUND 归为 not_found", () => {
    expect(toUserFacingError("ITEM_NOT_FOUND:abc").category).toBe("not_found");
  });

  it("把 authoring_v2_export_blocked 归为 validation", () => {
    const result = toUserFacingError("authoring_v2_export_blocked:missing_options");
    expect(result.category).toBe("validation");
    expect(result.userMessage).toContain("发布");
  });

  it("发布相关文案不再要求用户「先补齐」再发布", () => {
    for (const raw of [
      "authoring_v2_export_blocked:missing_options",
      "authoring_v2_export_blocked:quality_state=review_required",
      "authoring_v2_export_blocked:unresolved_answers=q1"
    ]) {
      expect(userMessageOf(raw)).not.toContain("请先补齐");
    }
  });

  it("发布与题库保存的新机器码都有人话", () => {
    expect(userMessageOf("PUBLISH_FORCE_OVERRIDE_INVALID:confirmedAt:bad")).toBe("发布请求无效，请重新点击发布。");
    expect(userMessageOf("PUBLISH_DUPLICATE_EXAM_ID:exam-1")).toContain("编号重复");
    expect(userMessageOf("nas_package_v2_lock_busy:os error 33")).toContain("另一次发布");
    expect(userMessageOf("ITEM_SOURCE_PURGED_AFTER_PUBLISH:item-1")).toBe(
      "原文件已在发布后删除，不能重新识别；题目仍可编辑、保存和发布。"
    );
  });

  it("把 requires_tauri_runtime 归为 runtime", () => {
    expect(toUserFacingError("requires_tauri_runtime").category).toBe("runtime");
  });

  it("把 source_file_too_large 归为 too_large", () => {
    expect(toUserFacingError("source_file_too_large:12345").category).toBe("too_large");
  });

  it("把权限类英文串归为 permission", () => {
    expect(toUserFacingError("EACCES: permission denied").category).toBe("permission");
    expect(toUserFacingError("filesystem is read-only").category).toBe("permission");
  });

  it("把 library_v2_tx 前缀归为 unknown 且给出可重试文案", () => {
    const result = toUserFacingError("library_v2_tx_commit_failed");
    expect(result.category).toBe("unknown");
    expect(result.userMessage).toContain("保存");
  });
});

describe("toUserFacingError — publish_check_failed 的结构化文案", () => {
  it("提取 blockers 中第一条 userMessage", () => {
    const payload = JSON.stringify({
      blockers: [
        { userMessage: "第 3 题缺少选项。" },
        { userMessage: "第 4 题没有答案。" }
      ]
    });
    const result = toUserFacingError(`publish_check_failed:${payload}`);
    expect(result.category).toBe("validation");
    expect(result.userMessage).toBe("第 3 题缺少选项。");
    expect(result.code).toBe("publish_check_failed");
  });

  it("payload 无 userMessage 时退回通用发布提示", () => {
    const result = toUserFacingError(`publish_check_failed:${JSON.stringify({ blockers: [{ code: "X" }] })}`);
    expect(result.category).toBe("validation");
    expect(result.userMessage).toContain("未完成");
  });

  it("payload 非法 JSON 时不抛出，退回通用发布提示", () => {
    const result = toUserFacingError("publish_check_failed:{not json");
    expect(result.category).toBe("validation");
    expect(result.userMessage).toContain("未完成");
    expect(result.internalDetail).toBe("publish_check_failed:{not json");
  });
});

describe("toUserFacingError — 透传与兜底", () => {
  it("已是中文人话的错误原样透传", () => {
    const message = "这个选项已用作本题答案。";
    const result = toUserFacingError(message);
    expect(result.userMessage).toBe(message);
    expect(result.internalDetail).toBe(message);
  });

  it("无法识别的纯 ASCII 机器串用兜底文案，但保留 internalDetail", () => {
    const raw = "weird_internal_failure_mode";
    const result = toUserFacingError(raw);
    expect(result.category).toBe("unknown");
    expect(result.userMessage).toBe("操作没有完成，请稍后重试。");
    expect(result.internalDetail).toBe(raw);
  });

  it("支持自定义兜底文案", () => {
    expect(toUserFacingError("opaque_ascii_code", "导入失败，请重试。").userMessage).toBe("导入失败，请重试。");
  });

  it("接受非 Error 值（字符串 / 对象）", () => {
    expect(toUserFacingError("ITEM_NOT_FOUND").category).toBe("not_found");
    expect(toUserFacingError({ toString: () => "ITEM_NOT_FOUND" }).category).toBe("not_found");
  });

  it("userMessageOf 只返回用户文案", () => {
    expect(userMessageOf(new Error("EDIT_VERSION_CONFLICT"))).toContain("刷新");
    expect(userMessageOf("plain_ascii_code")).toBe("操作没有完成，请稍后重试。");
  });
});

describe("toUserFacingError — 云端候选与发布门禁文案", () => {
  it("过期候选给出「重新检查」的出路，而不是泛化失败", () => {
    const result = toUserFacingError(new Error("LLM_SUGGESTION_STALE:current=5:base=3"));
    expect(result.category).toBe("conflict");
    expect(result.userMessage).toContain("重新运行云端检查");
  });

  it("权威稿已是 V2 时明确说明走新版流程", () => {
    const result = toUserFacingError(new Error("LLM_SUGGESTION_AUTHORITATIVE_STORE_IS_V2"));
    expect(result.userMessage).toContain("新版编辑流程");
  });

  it("发布门禁按具体原因给出可操作文案", () => {
    expect(userMessageOf("authoring_v2_export_blocked:human_verification_required")).toContain("逐题确认");
    expect(userMessageOf("authoring_v2_export_blocked:source_review_stale")).toContain("复核");
    expect(userMessageOf("authoring_v2_export_blocked:quality_state=blocked")).toContain("问题列表");
    expect(userMessageOf("authoring_v2_export_blocked:unresolved_answers=q14,q15")).toContain("没有答案");
    expect(userMessageOf("authoring_v2_export_blocked:ai_fallback=a.b.c")).toContain("手动补齐");
    // 未知原因仍然有兜底，不会把机器码当人话输出。
    // （旧的「请先补齐…再次发布」已移除：产品发布按钮一次点击即发布，不再有发布前补齐的要求。）
    expect(userMessageOf("authoring_v2_export_blocked:unknown_reason")).toContain("未完成");
    expect(userMessageOf("authoring_v2_export_blocked:unknown_reason")).not.toContain("authoring_v2");
  });
});
