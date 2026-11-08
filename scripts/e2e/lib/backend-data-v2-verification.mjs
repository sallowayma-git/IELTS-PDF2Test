// Strict assertions against real Tauri IPC after the UI imports real Electron snapshots.
export async function verifyBackendDataV2(session,{paper,attempts,exportPath,assert}) {
  const invoke=async(name,args)=>{const result=await session.invoke(name,args);if(!result?.ok)throw new Error(`${name}: ${JSON.stringify(result)}`);return result.value;};
  const details=[];
  for(const attempt of attempts){
    const detail=await invoke('get_attempt_detail',{attemptId:attempt.attemptId});details.push(detail);
    assert(`${attempt.attemptId}-content`,detail.passages.length===3 && detail.passages.every(p=>p.passage && p.answers.every(a=>a.context?.taskGroups?.length)),JSON.stringify(detail.anomalies));
    assert(`${attempt.attemptId}-events`,detail.events.some(e=>e.event_type==='answer_changed'),`events=${detail.events.length}`);
    assert(`${attempt.attemptId}-writing`,detail.writings.some(w=>w.content?.includes('writing submission')),`writings=${detail.writings.length}`);
    assert(`${attempt.attemptId}-listening`,detail.listening.some(l=>l.answers.length>0),`listening=${detail.listening.length}`);
  }
  const chosen=details.find(d=>d.passages.some(p=>p.answers.some(a=>a.is_correct===0)));
  if(!chosen)throw new Error('缺少可验证改判的原始错题');
  const passage=chosen.passages.find(p=>p.answers.some(a=>a.is_correct===0));
  const answer=passage.answers.find(a=>a.is_correct===0);
  const id=chosen.attempt.attempt_id;
  const before=await invoke('analytics_exam',{batchId:paper.batchId,paperId:paper.paperId});
  await invoke('create_score_override',{input:{attemptId:id,modality:'reading',passageIndex:passage.passage_index,responseSlotId:answer.response_slot_id,newIsCorrect:true,newScore:answer.max_score,reason:'端到端验收改判',expectedVersion:chosen.attempt.version}});
  const after=await invoke('get_attempt_detail',{attemptId:id});
  const changed=after.passages.find(p=>p.passage_index===passage.passage_index).answers.find(a=>a.response_slot_id===answer.response_slot_id);
  assert('override-preserves-raw',changed.is_correct===0 && changed.effectiveIsCorrect===true,JSON.stringify(changed));
  const exam=await invoke('analytics_exam',{batchId:paper.batchId,paperId:paper.paperId});
  assert('statistics-changed',JSON.stringify(exam.score)!==JSON.stringify(before.score),'有效改判应改变成绩统计');
  const writing=chosen.writings[0];if(!writing)throw new Error('缺少作文');
  const grade=await invoke('save_writing_grade',{input:{attemptId:id,writingSubmissionId:writing.writing_submission_id,expectedVersion:0,taTr:6.5,cc:7,lr:6,gra:7.5,overallComment:'完整链路批改'}});
  const reviewed=await invoke('review_writing_grade',{input:{gradeId:grade.grade_id,expectedVersion:grade.version}});
  await invoke('publish_writing_grade',{input:{gradeId:grade.grade_id,expectedVersion:reviewed.version}});
  const published=await invoke('get_attempt_detail',{attemptId:id});
  assert('writing-published',published.writings[0].grades.some(g=>g.status==='published' && g.task_band===6.75),'四维等权计算与发布');
  const exported=await invoke('analytics_export_xlsx',{batchId:paper.batchId,paperId:paper.paperId});
  assert('export-v2',exported.sheets.includes('改判后成绩') && exported.sheets.includes('写作成绩') && exported.sheets.includes('听力'),exportPath);
  const audit=await invoke('list_audit_log',{filter:{targetType:'attempt',targetId:id}});
  assert('audit',audit.audit.some(a=>a.action==='create_score_override') && audit.audit.some(a=>a.action==='publish_writing_grade'),`N=${audit.n}`);
}
