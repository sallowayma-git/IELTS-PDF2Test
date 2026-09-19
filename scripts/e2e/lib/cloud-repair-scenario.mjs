/**
 * 云端修复场景的**期望值装载 + 场景装配**。
 *
 * ## 为什么这份文件被重写过
 *
 * 旧版本是「脚本派生期望值」：脚本用正则从本地稿里剥掉页脚残留，把结果同时当作
 * （a）候选样本的内容、（b）剧本里的 `fixedPromptText`、（c）断言时比的字符串。
 * 三者是同一个值 —— 于是「云端能依据原文件修正识别错误」这条结论**无法被证伪**：
 * 脚本把答案写进剧本，假模型照抄，断言等于剧本里的字符串。剧本里一次 `read_source`
 * 都没有，而网关 prompt 却写着 "so it matches the ORIGINAL FILE"。
 *
 * 现在：
 *   · 期望值只来自 `fixtures/golden/cloud-repair/*.annotation.json`（人工标注）；
 *   · 错误必须是**本地识别真实产生的**——脚本只做核对（`before !== 标注值` 就如实
 *     报「前提不成立」），绝不往稿子里注入错误；
 *   · 剧本里**没有**正确题面。受控服务必须自己调用 `read_source`，从它返回的原文
 *     文本里把题面取出来（见 `controlled-llm-service.mjs`）。
 *
 * ## 边界（必须如实写进报告）
 *
 *   · 候选样本是**合成**的：它证明「管线能把一条有出处的修改一路带到权威稿/画布/导出」，
 *     不证明「某个真实模型能读对这份 PDF」。唯一被真正验证的是：受控服务给出的
 *     **修改内容与证据引文**都必须来自 `read_source` 返回的原文，而不是剧本常量。
 *   · 剧本里没有任何一个答案值：这份原文件没有答案页，答案**不能**被编造出来。
 */

import fs from 'node:fs';
import path from 'node:path';

/** 节点树里的全部文字（按出现顺序拼接）。 */
export function textOfNodes(nodes) {
  const out = [];
  const walk = (node) => {
    if (!node || typeof node !== 'object') return;
    if (Array.isArray(node)) {
      for (const item of node) walk(item);
      return;
    }
    if (node.type === 'text' && typeof node.text === 'string') out.push(node.text);
    for (const key of ['children', 'content']) {
      if (Array.isArray(node[key])) walk(node[key]);
    }
  };
  walk(nodes);
  return out.join(' ').replace(/\s+/g, ' ').trim();
}

/** 节点树里全部 `type === "text"` 的节点（返回引用，便于原地改写）。 */
export function collectTextNodes(nodes, out = []) {
  const walk = (node) => {
    if (!node || typeof node !== 'object') return;
    if (Array.isArray(node)) {
      for (const item of node) walk(item);
      return;
    }
    if (node.type === 'text' && typeof node.text === 'string') out.push(node);
    for (const key of ['children', 'content']) {
      if (Array.isArray(node[key])) walk(node[key]);
    }
  };
  walk(nodes);
  return out;
}

/** 深拷贝（只走 JSON 数据，真实稿里没有别的东西）。 */
export function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

export const GOLDEN_RELATIVE_PATH = 'fixtures/golden/cloud-repair/demanding-reading-passage-3.annotation.json';

/**
 * 读入人工标注的 golden fixture。**这是本场景唯一的期望值来源。**
 *
 * 校验写在最前面：缺字段就当场炸，而不是等到断言阶段才发现「期望值是 undefined」——
 * 后者会让 `prompt === undefined` 这类比较静默失败。
 */
export function loadRepairGolden(repoRoot) {
  const fullPath = path.join(repoRoot, GOLDEN_RELATIVE_PATH);
  const golden = JSON.parse(fs.readFileSync(fullPath, 'utf8'));
  const problems = [];
  if (golden?.schemaVersion !== 'CloudRepairGoldenV1') {
    problems.push(`schemaVersion 必须是 CloudRepairGoldenV1，实际 ${JSON.stringify(golden?.schemaVersion)}`);
  }
  if (!Array.isArray(golden?.recognitionErrors) || golden.recognitionErrors.length === 0) {
    problems.push('recognitionErrors 不能为空');
  }
  for (const [index, entry] of (golden?.recognitionErrors ?? []).entries()) {
    for (const field of ['localDraftContains', 'originalFileSays', 'originalFileQuote']) {
      if (typeof entry?.[field] !== 'string' || entry[field].length === 0) {
        problems.push(`recognitionErrors[${index}].${field} 必须是非空字符串`);
      }
    }
    if (!Array.isArray(entry?.target?.slotIds) || entry.target.slotIds.length === 0) {
      problems.push(`recognitionErrors[${index}].target.slotIds 不能为空`);
    }
    if (!Number.isInteger(entry?.sourcePage?.oneBased) || entry.sourcePage.oneBased < 1) {
      problems.push(`recognitionErrors[${index}].sourcePage.oneBased 必须是 >= 1 的整数`);
    }
  }
  if (problems.length > 0) {
    throw new Error(`golden fixture 不合法（${fullPath}）：${problems.join('；')}`);
  }
  return { ...golden, path: fullPath };
}

/**
 * 把「真实本地稿 + golden 期望值」装配成候选样本与修复剧本。
 *
 * 返回 `{ ok: false, reason, ... }` 表示**场景前提不成立**（本地识别没有产出被标注的那个
 * 错误，或者真实稿里找不到承载它的作答组）。调用方应如实记 `not-executable`，
 * **不要**退化成一份通用样本硬跑，也不要往稿子里注入一个错误。
 */
export function deriveRepairScenario(draft, golden) {
  const annotated = golden?.recognitionErrors?.[0];
  if (!annotated) return { ok: false, reason: 'golden fixture 里没有标注任何识别错误' };
  const groups = Array.isArray(draft?.taskGroups) ? draft.taskGroups : [];
  if (groups.length === 0) return { ok: false, reason: '真实稿里没有任何题组' };

  // ── 1. 目标由**真实稿**定位：按 fixture 标注的 slotId 找承载它的作答组 ──
  // 刻意不按「题面里有没有页脚」去搜：那是旧版的派生方式，等于用期望值的特征
  // 去反向找目标，找到的必然是同一个东西，验证不了任何事。
  const wantedSlots = annotated.target.slotIds;
  let fix = null;
  for (const group of groups) {
    for (const response of group.responseGroups ?? []) {
      const slots = Array.isArray(response.slotIds) ? response.slotIds : [];
      if (!wantedSlots.every((slot) => slots.includes(slot))) continue;
      const textNodes = collectTextNodes(response.prompt ?? []);
      if (textNodes.length !== 1) continue;
      const anchor = (response.sourceAnchors ?? [])[0] ?? {};
      fix = {
        taskId: group.taskId,
        responseGroupId: response.responseGroupId,
        questionNumbers: slots.slice(),
        // 改前 = 真实稿此刻的内容（读出来的）；改后 = golden 标注的原文件真值。
        before: textNodes[0].text,
        after: annotated.originalFileSays,
        sourcePageOneBased: annotated.sourcePage.oneBased,
        sourcePageZeroBased: annotated.sourcePage.zeroBased ?? null,
        questionNumber: annotated.target.questionNumber,
        sourceFileId: anchor.sourceFileId ?? null,
        anchorPageIndex: anchor.pageIndex ?? null,
      };
      break;
    }
    if (fix) break;
  }
  if (!fix) {
    return {
      ok: false,
      reason: `真实稿里找不到承载 ${JSON.stringify(wantedSlots)} 的作答组`,
    };
  }

  // ── 2. 核对「错误是本地识别真实产生的」──
  // 这是本场景的地基：如果本地识别没有出错，那么「云端修正了识别错误」就无从谈起，
  // 脚本应当如实说不适用，而不是自己造一个错出来。
  if (fix.before !== annotated.localDraftContains) {
    return {
      ok: false,
      reason: '本地识别没有产出 golden fixture 标注的那个错误',
      observed: fix.before,
      expected: annotated.localDraftContains,
      target: { taskId: fix.taskId, responseGroupId: fix.responseGroupId, slotIds: fix.questionNumbers },
    };
  }

  // ── 3. 裁定目标：真实稿里的 YES/NO/NOT GIVEN 题组 ──
  // 候选把它的题干说明只抄了首句，当前稿是完整的 → 云端应裁定「当前稿对」。
  const ruleTarget =
    groups.find((group) => (group.taskType ?? '').includes('yes_no_not_given'))
    ?? groups.find((group) => textOfNodes(group.instructions ?? []).length > 80)
    ?? groups[0];
  const ruleBefore = textOfNodes(ruleTarget.instructions ?? []);
  const ruleNodes = collectTextNodes(ruleTarget.instructions ?? []);
  if (!ruleBefore || ruleNodes.length !== 1) {
    return { ok: false, reason: '真实稿里找不到一个可用于裁定的题组说明' };
  }
  const ruleCandidateText = ruleBefore.split(/(?<=[?.])\s+/u)[0] ?? ruleBefore;
  if (ruleCandidateText === ruleBefore) {
    return { ok: false, reason: '候选侧的说明截断后与原文相同，构不成一条差异' };
  }

  // ── 4. 候选样本：整卷照抄真实稿，只改被标注的那一处 + 裁定靶子 ──
  // 「云端对原文件的独立识别」在这里被建模为：题面 = golden 标注的原文真值。
  const candidate = {
    passage: clone(draft.passage ?? {}),
    taskGroups: clone(groups),
    answerSlots: clone(draft.answerSlots ?? {}),
    answerKey: clone(draft.answerKey ?? {}),
    unresolvedRegions: [],
    sourceCoverageNotes: [],
  };
  const candidateFixGroup = candidate.taskGroups.find((group) => group.taskId === fix.taskId);
  const candidateFixResponse = (candidateFixGroup?.responseGroups ?? []).find(
    (response) => response.responseGroupId === fix.responseGroupId,
  );
  collectTextNodes(candidateFixResponse.prompt ?? [])[0].text = fix.after;
  const candidateRuleGroup = candidate.taskGroups.find((group) => group.taskId === ruleTarget.taskId);
  collectTextNodes(candidateRuleGroup.instructions ?? [])[0].text = ruleCandidateText;

  // ── 5. 模型确实无法定论的疑问：从真实稿里**读出来**的不一致，不是编出来的 ──
  const unresolved = [];
  for (const group of groups) {
    const instructions = textOfNodes(group.instructions ?? []);
    const wantsLetterList = /list of words and phrases|A\s*-\s*H|A\s*–\s*H/iu.test(instructions);
    if (!wantsLetterList) continue;
    if (group.optionBank) continue;
    const numbers = group.displayRange
      ? `${group.displayRange.start}-${group.displayRange.end}`
      : group.taskId;
    unresolved.push({
      targetId: group.taskId,
      message:
        `第 ${numbers} 题的题干说明要求从 A–H 词表里选词，但当前稿没有对应的选项库，`
        + '作答被当成自由输入。原文件里确实印着这份词表，可是「要不要按匹配题重建词表」'
        + '会改变这道题的作答与判分方式，属于编辑决策，云端不能替用户定。',
      pageIndex: (group.sourceAnchors ?? [])[0]?.pageIndex ?? 1,
      quote: 'Complete the summary using the list of words and phrases',
    });
    break;
  }

  const plan = {
    _comment:
      '受控模型服务的修复剧本。**刻意不含任何期望值**：没有 fixedPromptText、没有正确题面。'
      + '受控服务必须自己 read_source，从返回的原文文本里取出题面与引文。',
    // 目标定位：给的是 slotId（golden 标注），受控服务在**真实稿**里自己找承载它的作答组。
    fixSlotIds: wantedSlots.slice(),
    questionNumber: annotated.target.questionNumber,
    // 原文页号（1-based，与 read_source 返回的页对象一致）。
    sourcePageOneBased: annotated.sourcePage.oneBased,
    // 裁定：给目标与「去哪一行找依据」，引文本身必须从 read_source 的返回里读。
    rulings: [
      {
        targetType: 'task_group',
        targetId: ruleTarget.taskId,
        field: 'instructions',
        ruling: 'current_is_correct',
        reason: '原文件里这段说明包含完整的 YES / NO / NOT GIVEN 定义，当前稿与之一致，候选只抄了首句。',
        // 只是「在原文里定位到哪一行」的检索键，**不是引文内容**；
        // 引文必须由受控服务从 read_source 的返回里逐字取出。
        evidenceKeyword: 'NOT GIVEN',
      },
    ],
    unresolved,
    finishNote: '受控服务：已按原文件修正题面残留，并裁定一条候选读错的差异；无法定论的疑问已如实交出。',
  };

  return {
    ok: true,
    candidate,
    plan,
    fix,
    rule: {
      taskId: ruleTarget.taskId,
      before: ruleBefore,
      candidate: ruleCandidateText,
    },
    golden: {
      path: golden.path,
      fixtureId: golden.fixtureId,
      errorId: annotated.id,
      errorClass: annotated.errorClass,
      originalFileSays: annotated.originalFileSays,
      originalFileQuote: annotated.originalFileQuote,
      localDraftContains: annotated.localDraftContains,
    },
  };
}
