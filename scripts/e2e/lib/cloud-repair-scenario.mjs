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
 *
 * ## 答案主张（`claim` / `plan.answerClaim`）
 *
 * 题面类差异的原文行就在题组自己的锚点页上，包模式第一轮天然自带——CDP 步骤 11b
 * 「至少一个包走了 L1」在这类场景下**结构性**无法满足。本函数因此再构造一条候选侧的
 * 「答案主张」差异：候选（云端独立识别的建模）给一个 golden 标注了 `answerKeyAbsence`
 * 的无答案槽位声明一个答案（主张值取自主张组自己的文字，不引入常量）。这份原文件
 * 没有答案页 ⇒ 包里也没有 ⇒ 受控服务必须抓取（`read_source`，L1）去核实，核实不了
 * 就如实交还用户、**绝不应用**。主张值不出现在剧本里（反自证守卫），权威稿里该槽位
 * 仍是 unresolved（由链路步骤 15 证明）。
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

  // ── 3′. 答案主张靶子：候选（云端独立识别的建模）给一个无答案的槽位声明一个答案 ──
  //
  // 为什么需要它：步骤 11b 验收「至少一个包走了 L1」在**题面类**场景下结构性无法满足——
  // 题面的原文行就在题组自己的锚点页上，包第一轮天然自带，升级永远不会发生（实测
  // 3 个包的升级级别全是 0，其中 2 个第一次请求就带着承载正确答案的第 4 页）。
  // 这份卷子的 golden 明确标注了 `answerKeyAbsence`：原文件**没有答案页**。于是
  // 「候选声明一个答案」正好构成一条**修正页落在包范围之外**的答案类差异：
  // 包切分时会去找答案页（`locate_answer_pages`），找不到 ⇒ 包里没有答案页且
  // `answerPagesUnknown: true` ⇒ 模型必须抓取（`read_source`，L1 的一条腿）去核实，
  // 抓完发现原文件确实没有答案行 ⇒ 无法核实，**不应用**，把主张如实交还用户。
  //
  // 纪律与裁定靶子同构：主张是**候选侧**的建模（受控服务扮演云端），不是往本地稿里
  // 注入错误；主张值从本地稿自身内容里派生（题组自己的文字里取词），不引入常量；
  // 剧本里**绝不**出现主张值（反自证守卫在下面），受控服务也**绝不**应用它——
  // 链路步骤 15 会证明权威稿里这个槽位仍是 unresolved（云端不得编造答案）。
  //
  // 刻意**跳过**修复组与裁定组：那两个包的剧本行为（read_source 改题面 / record_ruling）
  // 必须保持现状，L1 由主张包自己走出。
  const claimWordCandidates = [];
  for (const group of groups) {
    if (group.taskId === fix.taskId) continue;
    if (group.taskId === ruleTarget.taskId) continue;
    const slotIds = (group.responseGroups ?? []).flatMap((response) => (Array.isArray(response.slotIds) ? response.slotIds : []));
    const slotId = slotIds.find((slot) => {
      const answer = draft?.answerKey?.[slot];
      return !answer || answer?.kind === 'unresolved';
    });
    if (!slotId) continue;
    const words = (textOfNodes(group.stimulus ?? group.instructions ?? []) ?? '')
      .split(/\s+/u)
      .map((word) => word.toLowerCase())
      .filter((word) => /^[a-z]{5,}$/u.test(word));
    claimWordCandidates.push({ taskId: group.taskId, slotId, questionNumber: Number((slotId.match(/\d+/u) ?? [])[0] ?? 0), words });
    break;
  }
  // 主张组（至多一个）在这里定位；主张**值**要等剧本成型之后才选（见第 6 节）——
  // 剧本里出现主张值，受控服务就不用抓取核实了，场景退回自证。
  const claimTarget = claimWordCandidates[0] ?? null;

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

  // ── 6. 答案主张：值在剧本成型**之后**才选，且不得出现在剧本里 ──
  // 剧本（含裁定理由、疑问文案）里出现主张值，受控服务就不用抓取核实了，场景退回自证。
  // 主张词选自主张组自己的文字（真实稿内容，不是常量）。
  let claim = null;
  if (claimTarget) {
    const serializedPlan = JSON.stringify(plan);
    const claimWord = (claimTarget.words ?? []).find((word) => !serializedPlan.includes(word)) ?? null;
    if (!claimWord) {
      return {
        ok: false,
        reason: '主张组的文字里选不出一个不出现在剧本里的词，答案主张构造不了',
        target: { taskId: claimTarget.taskId, slotId: claimTarget.slotId },
      };
    }
    candidate.answerKey[claimTarget.slotId] = {
      kind: 'text',
      values: [claimWord],
      normalization: 'ielts_default',
    };
    plan.answerClaim = {
      slotId: claimTarget.slotId,
      questionNumber: claimTarget.questionNumber,
      grabTool: 'read_source',
      // 原文没有答案页（golden 的 answerKeyAbsence）；抓**最后一页**核实——那是最可能
      // 印答案页的地方。抓完没有答案行 ⇒ 无法核实 ⇒ 交还用户，绝不应用。
      searchPages: [Number(golden?.source?.pageCount) || 5].filter((page) => page >= 1),
      unresolvedMessage:
        `第 ${claimTarget.questionNumber} 题的答案无法核实：原文件里没有答案页（抓取核对过），`
        + '云端不能编造答案。请对照原文件或自行填写。',
      finishNote: '受控服务：答案主张无法在原文件里核实，已如实交还用户',
    };
    if (JSON.stringify(plan).includes(claimWord)) {
      return {
        ok: false,
        reason: '剧本里出现了答案主张的值：受控服务就不用抓取核实了，场景退回自证',
        target: { taskId: claimTarget.taskId, slotId: claimTarget.slotId },
      };
    }
    claim = {
      taskId: claimTarget.taskId,
      slotId: claimTarget.slotId,
      questionNumber: claimTarget.questionNumber,
      searchPages: plan.answerClaim.searchPages,
    };
  }

  return {
    ok: true,
    candidate,
    plan,
    fix,
    claim,
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

/**
 * 答案类场景（P10）的**期望值装载 + 场景装配**。
 *
 * ## 场景定义
 *
 * 某题的答案错了（本地识别真实产生的错误），而正确答案所在的**答案页**不在该题组的
 * 锚点页上。包模式第一轮拿不到答案页 ⇒ 模型必须先 `report_insufficient_context`
 * 或用抓取工具（`read_source`）把答案页取回来，**之后**才能改对。
 * 这正是 CDP 步骤 11b「至少一个包走了 L1」在这份场景下可满足的形状——
 * 题面类场景的原文行就在题组自己的锚点页上，包天然自足，升级永远不会发生。
 *
 * ## 期望值来源与反自证（与题面类场景同一条纪律）
 *
 *   · 期望值只来自 golden fixture 新增的 `answerErrors` 标注（人工核对答案页后写下）；
 *   · 错误必须是本地识别真实产生的：`draft.answerKey[slotId]` 的当前值 ≠ 标注的
 *     原文件真值，否则场景前提不成立；
 *   · **剧本里没有任何答案值**：plan 只带「改哪个槽、答案印在哪一页、用哪个抓取工具」。
 *     候选样本带真值（候选本就是「云端独立识别」的建模），但受控服务的修复依据是
 *     **抓取回来的原文行**（如 `14 A`），不是候选切片里的值；
 *   · `plan` 与 `answerErrors[].originalAnswer` 的包含关系是本场景的反自证守卫，
 *     由派生函数自己检查，`plan` 里出现答案值直接判前提不成立。
 *
 * ## 为什么当前仓库里这条场景是 not-executable
 *
 * 唯一的 golden（demanding-reading-passage-3）明确标注了 `answerKeyAbsence`：
 * 那份原文件**没有答案页**。要跑答案类场景，需要一份带答案页的原文件 + 对应的
 * `answerErrors` 人工标注。在此之前本函数如实返回 `ok:false` 并给出原因——
 * 这不是失败，也不是通过，是「前提不成立」。
 */
export function deriveAnswerRepairScenario(draft, golden) {
  const entries = Array.isArray(golden?.answerErrors) ? golden.answerErrors : [];
  if (entries.length === 0) {
    return {
      ok: false,
      reason:
        'golden fixture 没有标注答案类错误（answerErrors）：这份卷子派生不出「答案页不在锚点页上」的场景',
      fixtureId: golden?.fixtureId ?? null,
    };
  }
  const annotated = entries[0];
  for (const field of ['slotIds', 'questionNumber', 'localAnswer', 'originalAnswer', 'answerPage']) {
    if (annotated?.[field] === undefined || annotated?.[field] === null) {
      return { ok: false, reason: `answerErrors[0].${field} 缺失：答案类场景的标注不完整` };
    }
  }
  const answerPageOneBased = Number(annotated.answerPage?.oneBased);
  if (!Number.isInteger(answerPageOneBased) || answerPageOneBased < 1) {
    return { ok: false, reason: 'answerErrors[0].answerPage.oneBased 必须是 >= 1 的整数' };
  }

  const groups = Array.isArray(draft?.taskGroups) ? draft.taskGroups : [];
  const wantedSlots = Array.isArray(annotated.slotIds) ? annotated.slotIds : [];
  let group = null;
  for (const candidate of groups) {
    const owned = (candidate.answerSlots ?? []).length > 0;
    const responseSlotIds = (candidate.responseGroups ?? [])
      .flatMap((response) => (Array.isArray(response.slotIds) ? response.slotIds : []));
    if (wantedSlots.every((slot) => responseSlotIds.includes(slot))) { group = candidate; break; }
    if (owned && wantedSlots.every((slot) => slot in (draft.answerKey ?? {}))) { group = group ?? candidate; }
  }
  if (!group) {
    return { ok: false, reason: `真实稿里找不到承载 ${JSON.stringify(wantedSlots)} 的题组` };
  }

  // 场景的定义性前提：答案页不在该题组的锚点页上。锚点的 pageIndex 是 0-based
  // （SourceAnchorV2），答案页给的是 1-based——与 read_source / 包 scope 的口径一致。
  const anchorPagesOneBased = (group.sourceAnchors ?? [])
    .map((anchor) => Number(anchor?.pageIndex ?? 0) + 1)
    .filter((page) => page >= 1);
  if (anchorPagesOneBased.includes(answerPageOneBased)) {
    return {
      ok: false,
      reason: `答案页（第 ${answerPageOneBased} 页）就在题组锚点页上：包第一轮就会带着它，派生不出「需要抓取」的场景`,
      anchorPagesOneBased,
    };
  }

  // 核对「答案错误是本地识别真实产生的」：当前稿的答案值必须等于标注的 localAnswer。
  const slotId = wantedSlots[0];
  const currentLabels = draft?.answerKey?.[slotId]?.labels ?? null;
  const normalized = (value) => JSON.stringify(Array.isArray(value) ? value.slice().sort() : value);
  if (!currentLabels || normalized(currentLabels) !== normalized(annotated.localAnswer)) {
    return {
      ok: false,
      reason: '本地识别没有产出标注的那个答案错误（当前稿的答案值与标注不符）',
      observed: currentLabels,
      expected: annotated.localAnswer,
      slotId,
    };
  }

  // 候选样本：整卷照抄真实稿，只把该题答案改成云端独立识别的真值（= 标注值）。
  // 与题面类场景同构：候选带真值、剧本不带，受控服务的依据必须是抓回的原文行。
  const candidate = {
    passage: clone(draft.passage ?? {}),
    taskGroups: clone(groups),
    answerSlots: clone(draft.answerSlots ?? {}),
    answerKey: clone(draft.answerKey ?? {}),
    unresolvedRegions: [],
    sourceCoverageNotes: [],
  };
  candidate.answerKey[slotId] = {
    ...clone(draft.answerKey?.[slotId] ?? {}),
    labels: clone(annotated.originalAnswer),
  };

  const plan = {
    _comment:
      '答案类场景的修复剧本。**刻意不含答案值**：originalAnswer 只进候选样本，'
      + '受控服务必须先抓取答案页（answerFetch: read_source），从返回的原文行（如「14 A」）里读出答案与引文。',
    fixSlotIds: wantedSlots.slice(),
    questionNumber: annotated.questionNumber,
    // 答案页号（1-based）：read_source / report_insufficient_context 的页口径。
    sourcePageOneBased: answerPageOneBased,
    answerFetch: 'read_source',
    rulings: [],
    unresolved: [],
    finishNote: `受控服务：第 ${annotated.questionNumber} 题答案已按抓取到的答案页改正`,
  };
  // 反自证守卫：剧本里出现答案值 ⇒ 受控服务不再需要抓取，场景退回自证。
  if (JSON.stringify(plan).includes(JSON.stringify(annotated.originalAnswer))) {
    return { ok: false, reason: '剧本里出现了答案值（originalAnswer）：受控服务就不再需要抓取答案页，场景退回自证' };
  }

  return {
    ok: true,
    kind: 'answer',
    candidate,
    plan,
    fix: {
      taskId: group.taskId ?? null,
      slotId,
      questionNumber: annotated.questionNumber,
      before: clone(annotated.localAnswer),
      after: clone(annotated.originalAnswer),
      answerPageOneBased,
      anchorPagesOneBased,
    },
    golden: {
      path: golden.path ?? null,
      fixtureId: golden.fixtureId ?? null,
      errorId: annotated.id ?? null,
    },
  };
}
