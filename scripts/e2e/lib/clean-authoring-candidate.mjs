/** Test-only clean candidate: preserve original question content and supply human inferred answers. */
import fs from 'node:fs';
import path from 'node:path';
import { collectTextNodes, textOfNodes } from './cloud-repair-scenario.mjs';

export const TEST_INFERRED_ANSWERS_PATH = 'fixtures/golden/cloud-repair/demanding-reading-passage-3.test-inferred-answers.json';

export function loadTestInferredAnswers(repoRoot) {
  const fixture = JSON.parse(fs.readFileSync(path.join(repoRoot, TEST_INFERRED_ANSWERS_PATH), 'utf8').replace(/^\uFEFF/u, ''));
  if (fixture.annotation?.kind !== 'human_inferred_test_only'
      || fixture.annotation?.scope !== 'clean_candidate_second_phase_only'
      || fixture.annotation?.hasOfficialAnswerPage !== false) {
    throw new Error('Clean candidate answers must be explicitly marked as test-only human inference without an official answer page');
  }
  return fixture;
}

function setSourceText(nodes, text, field) {
  const textNodes = collectTextNodes(nodes);
  if (!textNodes.length) throw new Error(`clean candidate 找不到 ${field} 的文字节点`);
  textNodes[0].text = text;
  for (const node of textNodes.slice(1)) node.text = '';
}

function removeQuestionAnchors(nodes, questionIds) {
  for (const node of nodes ?? []) {
    if (Array.isArray(node.sourceAnchors)) {
      node.sourceAnchors = node.sourceAnchors.filter((anchor) =>
        !(anchor.nodeIds ?? []).some((id) => questionIds.has(id)));
    }
    removeQuestionAnchors(node.children ?? node.content, questionIds);
  }
}

export function sanitizeCleanCandidate(seed, inferredAnswers, pdfPages) {
  const candidate = structuredClone(seed);
  const group2 = candidate.taskGroups.find((group) => group.taskId === 'group-2');
  if (!group2) throw new Error('clean candidate 找不到 group-2');
  const beforeInstructions = textOfNodes(group2.instructions);
  const pageThree = String(pdfPages.get(3) ?? '').normalize('NFKC');
  const groupStart = /Questions\s+32\s*[–—-]\s*35/u.exec(pageThree);
  if (!groupStart) throw new Error('原 PDF 第 3 页未能定位 Questions 32–35');
  const groupSource = pageThree.slice(groupStart.index);
  const numberedQuestions = [...groupSource.matchAll(/^\s*(3[2-5])\s+(?=\S)/gmu)];
  if (numberedQuestions.map((match) => Number(match[1])).join(',') !== '32,33,34,35') {
    throw new Error('原 PDF 第 3 页没有完整且顺序正确的 q32–q35 题干');
  }
  const originalInstructions = groupSource.slice(0, numberedQuestions[0].index).replace(/\s+/gu, ' ').trim();
  const questionIds = new Set();
  const restoredQuestions = [];
  const instructionAnchors = [ ...(group2.instructionSignature?.evidenceAnchors ?? []),
    ...(group2.instructions ?? []).flatMap((node) => node.sourceAnchors ?? []),
    ...collectTextNodes(group2.instructions).flatMap((node) => node.sourceAnchors ?? []) ];
  const questionStarts = numberedQuestions.map((match) => {
    const response = group2.responseGroups.find((entry) => entry.slotIds?.includes(`q${match[1]}`));
    const anchors = [...(response?.prompt ?? []), ...collectTextNodes(response?.prompt)]
      .flatMap((node) => node.sourceAnchors ?? []).filter((anchor) => anchor.pageIndex === 2 && Number.isFinite(anchor.bbox?.y));
    return anchors.length ? Math.min(...anchors.map((anchor) => anchor.bbox.y)) : null;
  });
  for (const [index, match] of numberedQuestions.entries()) {
    const slotId = `q${match[1]}`;
    const response = group2.responseGroups.find((entry) => entry.slotIds?.includes(slotId));
    if (!response) throw new Error(`clean candidate 找不到 ${slotId} 的题目`);
    const prompt = groupSource.slice(match.index + match[0].length,
      numberedQuestions[index + 1]?.index ?? groupSource.length).replace(/\s+/gu, ' ').trim();
    const before = textOfNodes(response.prompt);
    setSourceText(response.prompt, prompt, `${slotId} prompt`);
    // The local instruction region includes q35's wrapped last line. Transfer every
    // source anchor in each question's vertical interval before trimming instructions.
    const movedAnchors = instructionAnchors.filter((anchor) => anchor.pageIndex === 2
      && questionStarts[index] !== null && anchor.bbox?.y >= questionStarts[index]
      && anchor.bbox.y < (questionStarts[index + 1] ?? Infinity));
    for (const node of [...(response.prompt ?? []), ...collectTextNodes(response.prompt)]) {
      const anchors = [...(node.sourceAnchors ?? []), ...movedAnchors];
      node.sourceAnchors = [...new Map(anchors.map((anchor) => [JSON.stringify(anchor), anchor])).values()];
      for (const anchor of node.sourceAnchors ?? []) {
        for (const id of anchor.nodeIds ?? []) questionIds.add(id);
      }
    }
    restoredQuestions.push({ slotId, before, after: prompt, sourcePageOneBased: 3,
      transferredInstructionAnchorCount: movedAnchors.length });
  }
  // Reconstruct the source roles instead of paraphrasing the NOT GIVEN rule to dodge overlap.
  setSourceText(group2.instructions, originalInstructions, 'group-2 instructions');
  removeQuestionAnchors(group2.instructions, questionIds);
  if (group2.instructionSignature) {
    group2.instructionSignature.normalizedText = originalInstructions;
    group2.instructionSignature.evidenceAnchors = (group2.instructionSignature.evidenceAnchors ?? [])
      .filter((anchor) => !(anchor.nodeIds ?? []).some((id) => questionIds.has(id)));
  }

  const pageOne = String(pdfPages.get(1) ?? '').normalize('NFKC').replace(/\s+/gu, ' ').trim();
  const start = pageOne.indexOf('Ever since its elevation');
  const endToken = 'intrinsic interest of the topic.';
  const end = pageOne.indexOf(endToken, start);
  if (start < 0 || end < 0) throw new Error('原 PDF 第 1 页未能定位 passage 对齐段落');
  const passageNode = collectTextNodes(candidate.passage?.content)
    .find((node) => node.id === 'passage-paragraph-2-text');
  if (!passageNode) throw new Error('clean candidate 找不到 passage-paragraph-2-text');
  const passageBefore = passageNode.text;
  passageNode.text = pageOne.slice(start, end + endToken.length);

  const answerTruthUsed = [];
  candidate.answerKey ??= {};
  const entries = (inferredAnswers.groups ?? []).flatMap((group) => group.slots ?? []);
  const expected = Array.from({ length: 14 }, (_, index) => `q${index + 27}`);
  if (entries.map((entry) => `q${entry.question}`).join(',') !== expected.join(',')) {
    throw new Error('测试推断答案夹具必须且只能依次包含 q27–q40');
  }
  for (const entry of entries) {
    const slotId = `q${entry.question}`;
    const label = String(entry.answer ?? '').trim();
    const owner = candidate.taskGroups.find((group) => group.responseGroups?.some((response) => response.slotIds?.includes(slotId)));
    const response = owner?.responseGroups.find((group) => group.slotIds?.includes(slotId));
    const options = [...(owner?.optionBank?.options ?? []), ...(response?.options ?? [])];
    if (!label || !options.some((option) => option.label === label)) {
      throw new Error(`${slotId} 测试推断答案 ${label} 不在原卷选项中`);
    }
    candidate.answerKey[slotId] = { kind: 'option', labels: [label], assignment: 'per_slot' };
    answerTruthUsed.push({ slotId, answer: label, annotationKind: 'human_inferred_test_only' });
  }
  return {
    candidate,
    group2InstructionChanged: textOfNodes(group2.instructions) !== beforeInstructions,
    group2StructureRestored: true,
    restoredQuestions,
    passageTextChanged: passageNode.text !== passageBefore,
    answerTruthSource: TEST_INFERRED_ANSWERS_PATH,
    answerTruthKind: 'human_inferred_test_only',
    hasOfficialAnswerPage: false,
    answerTruthUsed,
    missingAnswerTruth: [],
  };
}
