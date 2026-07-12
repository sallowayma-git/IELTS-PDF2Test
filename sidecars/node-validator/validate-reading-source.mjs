#!/usr/bin/env node
import fs from 'node:fs';

function issue(layer, path, message, severity = 'error') {
  return { issueId: `issue-${Math.random().toString(36).slice(2, 10)}`, severity, layer, path, message, fixHint: null };
}

const allowedKinds = new Set([
  'single_choice',
  'multi_choice',
  'true_false_not_given',
  'yes_no_not_given',
  'matching',
  'classification',
  'summary_completion',
  'table_completion',
  'diagram_completion',
  'short_answer',
  'sentence_completion',
]);

function attrs(tag) {
  const result = {};
  const pattern = /([:\w-]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g;
  let match;
  while ((match = pattern.exec(tag))) {
    const [, key, doubleQuoted, singleQuoted, bare] = match;
    if (!key || key === tag.split(/\s+/)[0].replace('<', '')) continue;
    result[key.toLowerCase()] = doubleQuoted ?? singleQuoted ?? bare ?? '';
  }
  return result;
}

function tags(html, name) {
  return [...String(html ?? '').matchAll(new RegExp(`<${name}\\b[^>]*>`, 'gi'))].map((match) => match[0]);
}

function controlTags(html) {
  return [
    ...tags(html, 'input'),
    ...tags(html, 'select'),
    ...tags(html, 'textarea'),
    ...[...String(html ?? '').matchAll(/<[^>]*\b(?:paragraph-dropzone|match-dropzone|drop-target-summary)\b[^>]*>/gi)].map((match) => match[0]),
  ];
}

function controlQuestionId(attributes) {
  return (
    attributes.name ||
    attributes['data-question'] ||
    attributes['data-question-id'] ||
    attributes['data-target'] ||
    (attributes.id?.endsWith('_input') ? attributes.id.slice(0, -6) : attributes.id)
  );
}

function hasCollectibleControl(html, qid) {
  return controlTags(html).some((tag) => controlQuestionId(attrs(tag)) === qid);
}

function hasValidDropzone(html, qid) {
  return [...String(html ?? '').matchAll(/<[^>]*\b(?:paragraph-dropzone|match-dropzone|drop-target-summary)\b[^>]*>/gi)]
    .map((match) => attrs(match[0]))
    .some((attributes) => controlQuestionId(attributes) === qid);
}

function hasInvalidDropzone(html) {
  return [...String(html ?? '').matchAll(/<[^>]*\b(?:paragraph-dropzone|match-dropzone|drop-target-summary)\b[^>]*>/gi)]
    .map((match) => attrs(match[0]))
    .some((attributes) => !controlQuestionId(attributes));
}

function validate(source) {
  const issues = [];
  if (source?.schemaVersion !== 'ReadingExamSourceV1') issues.push(issue('ReadingExamSourceV1', '$.schemaVersion', 'schemaVersion must be ReadingExamSourceV1'));
  if (!source?.examId) issues.push(issue('ReadingExamSourceV1', '$.examId', 'examId is required'));
  if (!source?.meta?.title) issues.push(issue('ReadingExamSourceV1', '$.meta.title', 'meta.title is required'));
  if (!Array.isArray(source?.passage?.blocks) || source.passage.blocks.length === 0) issues.push(issue('ReadingExamSourceV1', '$.passage.blocks', 'passage.blocks cannot be empty'));
  if (!Array.isArray(source?.questionGroups) || source.questionGroups.length === 0) issues.push(issue('ReadingExamSourceV1', '$.questionGroups', 'questionGroups cannot be empty'));
  const answerKey = source?.answerKey ?? {};
  if (!Object.keys(answerKey).length) issues.push(issue('ReadingExamSourceV1', '$.answerKey', 'answerKey is empty; unanswered questions will be exported without scoring data', 'warning'));
  const covered = new Set();
  for (const group of source?.questionGroups ?? []) {
    if (!allowedKinds.has(group.kind)) issues.push(issue('ReadingExamSourceV1', `$.questionGroups.${group.groupId}.kind`, `${group.kind} is not an allowed group kind`));
    if ((group.kind === 'matching' || group.kind === 'classification') && typeof group.allowOptionReuse !== 'boolean') {
      issues.push(issue('ReadingExamSourceV1', `$.questionGroups.${group.groupId}.allowOptionReuse`, 'matching/classification groups must explicitly set allowOptionReuse'));
    }
    for (const qid of group.questionIds ?? []) {
      covered.add(qid);
      if (!(qid in answerKey)) issues.push(issue('ReadingExamSourceV1', `$.answerKey.${qid}`, `${qid} is missing from answerKey and will be exported without scoring data`, 'warning'));
      const html = group.bodyHtml ?? '';
      if (!hasCollectibleControl(html, qid)) {
        issues.push(issue('DomProtocol', `$.questionGroups.${group.groupId}.bodyHtml`, `No collectible control found for ${qid}`));
      }
      if (html.includes('dropzone') || html.includes('drop-target')) {
        if (!hasValidDropzone(html, qid) && !hasCollectibleControl(html, qid)) {
          issues.push(issue('DomProtocol', `$.questionGroups.${group.groupId}.bodyHtml`, `No valid dropzone target found for ${qid}`));
        }
        if (hasInvalidDropzone(html)) {
          issues.push(issue('DomProtocol', `$.questionGroups.${group.groupId}.bodyHtml`, 'Dropzone is missing data-question/data-question-id/data-target or id fallback'));
        }
      }
    }
  }
  for (const qid of Object.keys(answerKey)) {
    if (!covered.has(qid)) issues.push(issue('ReadingExamSourceV1', `$.questionGroups`, `${qid} from answerKey is not covered by any question group`));
  }
  const order = source?.questionOrder ?? [];
  if (!Array.isArray(order) || order.length !== covered.size) {
    issues.push(issue('ReadingExamSourceV1', '$.questionOrder', 'questionOrder length must equal covered question count'));
  }
  for (const qid of order) {
    if (!covered.has(qid)) issues.push(issue('ReadingExamSourceV1', '$.questionOrder', `${qid} is not covered by any question group`));
    if (!source?.questionDisplayMap?.[qid]) issues.push(issue('ReadingExamSourceV1', `$.questionDisplayMap.${qid}`, `${qid} is missing original display number`));
  }
  const hasErrors = issues.some((item) => item.severity === 'error');
  return {
    passed: !hasErrors,
    layers: ['ReadingExamSourceV1', 'DomProtocol'].map((layer) => {
      const layerIssues = issues.filter((item) => item.layer === layer);
      const errorCount = layerIssues.filter((item) => item.severity === 'error').length;
      const warningCount = layerIssues.filter((item) => item.severity === 'warning').length;
      return { layer, passed: errorCount === 0, issueCount: layerIssues.length, errorCount, warningCount };
    }),
    issues,
    generatedAt: new Date().toISOString(),
  };
}

const input = process.argv[2];
if (!input) {
  console.error('usage: validate-reading-source.mjs <reading-source.json>');
  process.exit(2);
}
const source = JSON.parse(fs.readFileSync(input, 'utf8'));
const report = validate(source);
console.log(JSON.stringify(report, null, 2));
process.exit(report.passed ? 0 : 1);
