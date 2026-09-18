/**
 * 云端修复场景的**派生**逻辑（从真实本地稿派生候选样本与修复剧本）。
 *
 * 为什么必须派生，而不是写死一份样本：
 *   候选是「云端对原文件的独立识别」，它必须与**当前这份真实稿**逐字段可比。
 *   写死一份通用样本会得到成百上千条无意义差异，修复循环只能把预算烧在噪声上，
 *   验收结果也就无法回答「云端到底替用户了结了什么」。
 *   派生只改**被点名的字段**，其余整卷照抄真实稿——于是差异集合是**可枚举**的：
 *     1) 作答结构差异：候选把题面里的页脚残留去掉了，当前稿还带着 → 云端应自动改掉；
 *     2) 说明文字差异：候选把题干说明读错了，当前稿是对的 → 云端应裁定「当前稿对」。
 *
 * 这份派生**不修改**任何后端文件，也不改真实稿；它只产出一个候选样本与一份剧本。
 * 受控服务是真实模型的替身，剧本里所有**值**都来自真实稿本身；受控服务回不出它没读到的东西。
 *
 * 边界（必须如实写在报告里）：
 *   · 候选样本是**合成**的。它证明的是「管线能把一条有出处的修改一路带到权威稿/画布/导出」，
 *     不证明「某个真实模型能读对这份 PDF」。
 *   · 剧本里没有任何一个答案值：这份原文件没有答案页，答案**不能**被编造出来。
 */

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

const PAGE_FOOTER = /\s\d+\s+BLANK\s+PAGE\s*$/iu;

/**
 * 从真实稿派生「候选样本 + 修复剧本」。
 *
 * 返回 `null` 表示这份稿子不满足场景前提（例如题面里没有可辨认的页脚残留），
 * 调用方应如实记 `not-executable`，**不要**退化成一份通用样本硬跑。
 */
export function deriveRepairScenario(draft) {
  const groups = Array.isArray(draft?.taskGroups) ? draft.taskGroups : [];
  if (groups.length === 0) return null;

  // ── 目标 1：题面里混进了页码/页脚（`14 BLANK PAGE`），这是真实存在的转录残留 ──
  let fix = null;
  for (const group of groups) {
    for (const response of group.responseGroups ?? []) {
      const textNodes = collectTextNodes(response.prompt ?? []);
      if (textNodes.length !== 1) continue;
      const text = textNodes[0].text;
      const matched = PAGE_FOOTER.exec(text);
      if (!matched) continue;
      const after = text.slice(0, matched.index).trim();
      if (!after) continue;
      fix = {
        taskId: group.taskId,
        responseGroupId: response.responseGroupId,
        questionNumbers: (response.slotIds ?? []).slice(),
        before: text,
        after,
        footer: matched[0].trim(),
        pageIndex: (response.sourceAnchors ?? [])[0]?.pageIndex ?? null,
        sourceFileId: (response.sourceAnchors ?? [])[0]?.sourceFileId ?? null,
      };
      break;
    }
    if (fix) break;
  }
  if (!fix) return null;

  // ── 目标 2：让候选把某组题干说明**读错**，当前稿是对的 ──
  // 选一个有完整说明文字的题组；候选版本只留首句，丢掉真正的作答要求。
  const ruleTarget =
    groups.find((group) => (group.taskType ?? '').includes('yes_no_not_given'))
    ?? groups.find((group) => textOfNodes(group.instructions ?? []).length > 80)
    ?? groups[0];
  const ruleBefore = textOfNodes(ruleTarget.instructions ?? []);
  if (!ruleBefore) return null;
  const ruleCandidateText = ruleBefore.split(/(?<=[?.])\s+/u)[0] ?? ruleBefore;
  if (ruleCandidateText === ruleBefore) return null;
  const ruleNodes = collectTextNodes(ruleTarget.instructions ?? []);
  if (ruleNodes.length !== 1) return null;

  // ── 候选样本：整卷照抄真实稿，只改上面点名的两处 ──
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

  // ── 模型确实无法定论的疑问：从真实稿里**读出来**的不一致，不是编出来的 ──
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
    _comment: '受控模型服务的修复剧本：所有取值都来自真实稿与真实 observation。',
    fixTaskId: fix.taskId,
    fixResponseGroupId: fix.responseGroupId,
    fixedPromptText: fix.after,
    // 证据引文就是被去掉的那段页脚文字本身——它确实是原文件里的页脚，不是题面。
    evidenceQuote: fix.footer,
    rulings: [
      {
        targetType: 'task_group',
        targetId: ruleTarget.taskId,
        field: 'instructions',
        ruling: 'current_is_correct',
        reason: '原文件里这段说明包含完整的 YES / NO / NOT GIVEN 定义，当前稿与之一致，候选只抄了首句。',
        quote: ruleCandidateText.slice(0, 60),
      },
    ],
    unresolved,
    finishNote: '受控服务：已按原文件修正题面残留，并裁定一条候选读错的差异；无法定论的疑问已如实交出。',
  };

  return {
    candidate,
    plan,
    fix,
    rule: {
      taskId: ruleTarget.taskId,
      before: ruleBefore,
      candidate: ruleCandidateText,
    },
  };
}
