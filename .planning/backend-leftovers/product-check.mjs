import fs from 'node:fs';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {DatabaseSync} from 'node:sqlite';
import {repoRoot,assertBuildFresh,launchTauriAppCdp,writeReport,sleep} from '../../scripts/e2e/lib/tauri-cdp-harness.mjs';
import {seedReadingJob,waitForRows,confirmRowsOnImportPage,libraryRows,APP_SETTINGS_KEY,setRowPart,ensureAllCheckedStable} from '../../scripts/e2e/lib/paper-publish-steps.mjs';

const stamp=new Date().toISOString().replace(/[:.]/g,'-');
const runDir=path.join(repoRoot,'artifacts/e2e-cdp',`backend-leftovers-${stamp}`);
const dataDir=path.join(runDir,'appdata/data');
const exePath=path.join(repoRoot,'src-tauri/target/debug/ielts-author-studio.exe');
assertBuildFresh({exePath});
fs.mkdirSync(dataDir,{recursive:true});
const fixture=JSON.parse(fs.readFileSync(path.join(repoRoot,'contracts/fixtures/authoring-structure-parity.json'),'utf8'));
const matrix=structuredClone(fixture.matrixInitial);
matrix.passage.content=matrix.passage.content.filter(n=>n.id!=='diagram');
const target=seedReadingJob(dataDir,'matrix',matrix,stamp);
const ready=JSON.parse(fs.readFileSync(path.join(repoRoot,'fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json'),'utf8'));
const readings=[1,2,3].map(i=>seedReadingJob(dataDir,i,ready,stamp));
const seed=spawnSync('cargo',['test','--manifest-path','src-tauri/Cargo.toml','--lib','dump_backend_data_listening_fixture','--','--ignored','--nocapture'],{cwd:repoRoot,env:{...process.env,PDF2TEST_E2E_LISTENING_APP_ROOT:dataDir},encoding:'utf8'});
fs.writeFileSync(path.join(runDir,'listening-seed.log'),seed.stdout+seed.stderr);
if(seed.status!==0)throw new Error('listening seed failed');
const listening=JSON.parse(seed.stdout.split(/\r?\n/).find(l=>l.startsWith('E2E_LISTENING=')).slice('E2E_LISTENING='.length));
// Only the initial test fixture's scope is selected here. All deletion/renumber edits use real IPC.
const db=new DatabaseSync(path.join(dataDir,'authoring_hub.db'));
const ds=JSON.parse(db.prepare('SELECT canonical_ds_json FROM library_items_v2 WHERE id=?').get(listening.jobId).canonical_ds_json);
ds.listening.scope='partial_practice';
db.prepare('UPDATE library_items_v2 SET canonical_ds_json=? WHERE id=?').run(JSON.stringify(ds),listening.jobId);db.close();
const report={task:'backend-leftovers',evidenceLevel:'real Tauri / WebView2 CDP automation; structural actions via real IPC; UI save/restart/preview/publish',assertions:[],workarounds:[],runDir};
function check(id,ok,detail){report.assertions.push({id,ok,detail});console.log(ok?'PASS':'FAIL',id,JSON.stringify(detail));if(!ok)throw new Error(id);}
let session;
async function invoke(command,args){const r=await session.invoke(command,args);if(!r.ok)throw new Error(`${command}: ${r.error}`);return r.value;}
async function workspace(id){return invoke('get_workspace_item',{itemId:id});}
async function edit(id,commands){const w=await workspace(id);await invoke('apply_editor_commands',{input:{itemId:id,baseVersion:w.editVersion,requestId:`leftovers-${Date.now()}`,commands}});return workspace(id);}
try{
 session=await launchTauriAppCdp({exePath,runDir,quiet:true});
 await waitForRows(session,[target.jobId,...readings.map(r=>r.jobId),listening.jobId],90000);
 const appended=await edit(target.jobId,[fixture.extraCases[0].patches[0]]);
 check('matrix-single-slot-siblings',appended.ds.taskGroups[1].responseGroups.every(g=>g.slotIds.length===1),appended.ds.taskGroups[1].responseGroups);
 const renumbered=await edit(target.jobId,[{op:'renumberQuestions',scope:'exam'}]);
 check('renumber-preserves-identities',Object.keys(renumbered.ds.answerSlots).sort().join()==Object.keys(appended.ds.answerSlots).sort().join(),Object.keys(renumbered.ds.answerSlots));
 await session.evaluate(`window.location.hash=${JSON.stringify('#/items/'+target.jobId)}`);
 await session.waitFor(`!!document.querySelector('[data-testid="workspace-save"]')`,{timeoutMs:60000,label:'editor'});
 await session.clickSelector('[data-testid="workspace-save"]');
 await session.waitFor(`document.querySelector('[data-testid="workspace-save-state"]')?.textContent==='已保存'`,{timeoutMs:40000,label:'save'});
 await session.screenshot('matrix-renumber-saved');
 await session.close();
 session=await launchTauriAppCdp({exePath,runDir,quiet:true});
 const reopened=await workspace(target.jobId);
 check('restart-retains-numbering',JSON.stringify(reopened.ds.answerSlots)===JSON.stringify(renumbered.ds.answerSlots),Object.values(reopened.ds.answerSlots).map(s=>[s.slotId,s.questionNumber]));
 await session.evaluate(`window.location.hash=${JSON.stringify('#/items/'+target.jobId)}`);
 await session.waitFor(`!!document.querySelector('[data-testid="workspace-mode-student"]')`,{timeoutMs:60000,label:'reopened-editor'});
 await session.clickSelector('[data-testid="workspace-mode-student"]');
 await session.waitFor(`!!document.querySelector('[data-testid="exam-canvas-v2-student"]')`,{timeoutMs:60000,label:'preview'});
 const preview=await session.evaluate(`({error:document.querySelector('[data-testid="workspace-preview-error"]')?.textContent??null,numbers:[...new Set([...document.querySelectorAll('[data-testid="exam-canvas-v2-student"] .v2-slot-number')].map(n=>Number(n.textContent.trim())).filter(n=>n>0))].sort((a,b)=>a-b),matrices:document.querySelectorAll('[data-testid="exam-canvas-v2-student"] .v2-matching-matrix').length})`);
 check('preview-contiguous',preview.error===null&&JSON.stringify(preview.numbers)==='[1,2,3,4,5,6,7]',preview);
 check('preview-retains-matrix',preview.matrices===1,preview);
 await session.screenshot('student-preview-contiguous');
 const before=await workspace(listening.jobId);
 const part=before.ds.listening.parts[0];
 const emptied=await edit(listening.jobId,part.taskIds.map(taskId=>({op:'deleteTaskGroup',taskId})));
 const reason='第 1 部分没有题目，请删除该部分或补题后再发布';
 check('empty-part-saves',emptied.ds.listening.parts[0].taskIds.length===0&&emptied.ds.quality.issues.some(i=>i.code==='LISTENING_PART_EMPTY'&&i.message===reason),emptied.ds.quality.issues.filter(i=>i.code==='LISTENING_PART_EMPTY'));
 await session.evaluate(`window.location.hash=${JSON.stringify('#/items/'+listening.jobId)}`);
 await session.waitFor(`!!document.querySelector('[data-testid="workspace-save"]')`,{timeoutMs:60000,label:'empty-part-editor'});
 await session.screenshot('empty-part-editor');
 await confirmRowsOnImportPage(session,readings.map(r=>r.jobId));
 const nas=path.join(runDir,'empty-part-nas');
 await session.evaluate(`(()=>{const s=JSON.parse(localStorage.getItem(${JSON.stringify(APP_SETTINGS_KEY)})||'{}');s.nasDestination=${JSON.stringify(nas)};localStorage.setItem(${JSON.stringify(APP_SETTINGS_KEY)},JSON.stringify(s));window.location.hash='#/settings';return true})()`);
 await sleep(400);await session.evaluate(`window.location.hash='#/library?modality=reading'`);
 await session.waitFor(`!!document.querySelector('[data-testid="paper-table"]')`,{timeoutMs:40000,label:'paper-table'});
 for(const [i,r] of readings.entries()){
  await session.waitFor(`!!document.querySelector('[data-item-id="${r.jobId}"] [data-testid="library-row-part-select"]')`,{timeoutMs:30000,label:`reading-part-${i+1}`});
  check(`part-select-${i+1}`,await setRowPart(session,r.jobId,`P${i+1}`),r.jobId);
  const deadline=Date.now()+15000;
  while(Date.now()<deadline && (await libraryRows(session)).find(row=>row.id===r.jobId)?.partLabel!==`P${i+1}`)await sleep(200);
 }
 await ensureAllCheckedStable(session,readings.map(r=>r.jobId));
 await session.evaluate(`window.location.hash='#/library?modality=listening'`);
 await session.waitFor(`!!document.querySelector('tr[data-item-id="${listening.jobId}"] input[type="checkbox"]')`,{timeoutMs:30000,label:'listening-row'});
 await ensureAllCheckedStable(session,[listening.jobId]);
 await session.evaluate(`window.location.hash='#/library?modality=reading'`);
 await session.waitFor(`!!document.querySelector('tr[data-item-id="${readings[0].jobId}"] input[type="checkbox"]')`,{timeoutMs:30000,label:'back-to-reading'});
 const stable=await ensureAllCheckedStable(session,readings.map(r=>r.jobId));
 check('all-paper-reading-parts-selected',stable.stable,stable);
 await session.waitFor(`!!document.querySelector('[data-testid="paper-publish"]')&&!document.querySelector('[data-testid="paper-publish"]').disabled`,{timeoutMs:15000,label:'empty-part-publish-enabled'});
 await session.clickSelector('[data-testid="paper-publish"]');
 await session.waitFor(`!!document.querySelector('[data-testid="paper-confirm-ok"]')`,{timeoutMs:10000,label:'publish-confirm'});
 await session.clickSelector('[data-testid="paper-confirm-ok"]');
 await session.waitFor(`document.querySelector('[data-testid="paper-composer"] [role="status"]')?.textContent.includes(${JSON.stringify(reason)})`,{timeoutMs:30000,label:'publish-empty-part-reason'});
 const message=await session.evaluate(`document.querySelector('[data-testid="paper-composer"] [role="status"]').textContent`);
 check('ui-publish-blocked-in-chinese',message.includes(reason),message);
 check('nas-untouched',!fs.existsSync(nas),nas);
 await session.screenshot('empty-part-publish-blocked');
 report.verdict='passed';
}catch(e){report.verdict='failed';report.error=String(e.stack??e);console.error(e);if(session)await session.screenshot('failure').catch(()=>{});process.exitCode=1;}
finally{if(session)await session.close().catch(()=>{});console.log(writeReport(runDir,report));}
