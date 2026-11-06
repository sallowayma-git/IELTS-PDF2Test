// Real Tauri UI/IPC/scheduler/SQLite with a controlled HTTP model service.
// Covers separate single PDFs and a whole PDF with repeated question numbers.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { DatabaseSync } from 'node:sqlite';
import { assertBuildFresh, launchTauriAppCdp, repoRoot, sha256File, sleep } from './lib/tauri-cdp-harness.mjs';

const runDir = path.join(repoRoot, 'artifacts/e2e-cdp', `run-cloud-multi-${new Date().toISOString().replace(/[:.]/g,'-')}`);
const data = path.join(runDir, 'appdata/data');
const exePath = path.join(repoRoot, 'src-tauri/target/debug/ielts-author-studio.exe');
fs.mkdirSync(path.join(data, 'config/secrets'), { recursive: true });
const report = { channel:'webview2-cdp', controlledModel:true, diagnosticPort:true, securityRelaxations:[],
  exeSha256:sha256File(exePath), build:assertBuildFresh({exePath}), requests:[], checks:[] };

function pdf(pages) {
  const objects = ['<< /Type /Catalog /Pages 2 0 R >>',
    `<< /Type /Pages /Kids [${pages.map((_,i)=>`${4+2*i} 0 R`).join(' ')}] /Count ${pages.length} >>`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'];
  pages.forEach((lines,i)=>{
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R >> >> /Contents ${5+2*i} 0 R >>`);
    const stream='BT /F1 10 Tf 16 TL 40 800 Td\n'+lines.map(line=>`(${line.replace(/[\\()]/g,'\\$&')}) Tj T*\n`).join('')+'ET';
    objects.push(`<< /Length ${Buffer.byteLength(stream)} >>\nstream\n${stream}\nendstream`);
  });
  let out='%PDF-1.4\n'; const offsets=[0];
  objects.forEach((object,i)=>{offsets.push(Buffer.byteLength(out));out+=`${i+1} 0 obj\n${object}\nendobj\n`;});
  const xref=Buffer.byteLength(out);
  return out+`xref\n0 ${offsets.length}\n0000000000 65535 f \n`+offsets.slice(1).map(offset=>`${String(offset).padStart(10,'0')} 00000 n \n`).join('')+`trailer\n<< /Size ${offsets.length} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;
}
const paragraph=(id,text)=>({type:'paragraph',id,children:[{type:'text',id:`${id}-text`,text}]});
function article(title, number, answer, category) {
  return { meta:{title,language:'en',category}, passage:{content:[paragraph('passage-A',`${title} describes a separate history of scientific discovery and its lasting effects on everyday life.`)],paragraphMap:{}},
    taskGroups:[{taskId:'group-1',taskType:'true_false_not_given',displayRange:{kind:'range',start:number,end:number},
      instructions:[paragraph('instruction',`Questions ${number} Do the following statements agree with the information in the passage? Write TRUE FALSE NOT GIVEN.`)],
      responseGroups:[{responseGroupId:'group-1-r1',kind:'choice',slotIds:[`q${number}`],prompt:[paragraph('prompt',`The ${title} article explains a scientific discovery.`)]}]}],
    answerSlots:{[`q${number}`]:{slotId:`q${number}`,questionNumber:number,displayLabel:String(number),hostType:'prompt',hostNodeId:'prompt',interaction:'radio',participation:'scoring',confidence:1}},
    answerKey:{[`q${number}`]:{kind:'option',labels:[answer],assignment:'per_slot'}},warnings:[] };
}
const specs=[
  {title:'Article Alpha',marker:'ARTICLE_ALPHA_SOURCE',number:1,answer:'TRUE',category:'P1',sourcePages:[1],answerPages:[2]},
  {title:'Article Beta',marker:'ARTICLE_BETA_SOURCE',number:14,answer:'FALSE',category:'P2',sourcePages:[3],answerPages:[4]},
  {title:'Article Gamma',marker:'ARTICLE_GAMMA_SOURCE',number:1,answer:'NOT GIVEN',category:'P1',sourcePages:[5],answerPages:[6]},
];
function sourcePage(spec) {
  return [spec.marker,`READING PASSAGE ${spec.category.slice(1)}`,spec.title,
    `${spec.title} describes a separate history of scientific discovery and its lasting effects on everyday life.`,
    `You should spend about 20 minutes on Questions ${spec.number}, which are based on Reading Passage ${spec.category.slice(1)}.`,
    `Questions ${spec.number}`, 'Do the following statements agree with the information given in the passage?',
    'Write TRUE FALSE NOT GIVEN.', `${spec.number} The article explains a scientific discovery.`];
}
const multiFile=path.join(runDir,'multi-tests.pdf');
fs.writeFileSync(multiFile,pdf(specs.flatMap(spec=>[sourcePage(spec),['Answer Key',`${spec.number} ${spec.answer}`]])));
const singleFiles=['Single Delta','Single Epsilon'].map((title,i)=>{
  const file=path.join(runDir,`single-${i}.pdf`);
  fs.writeFileSync(file,pdf([sourcePage({title,marker:`SINGLE_${i}_SOURCE`,number:1,category:'P1'})])); return file;
});
let childOnly = false;
let failNext = false;
let inflight=0, completed=0, maxInflight=0;
const server=http.createServer(async(req,res)=>{
  // Cache warm-up: no second model request may start before the first one ends.
  if(completed===0&&inflight>0)report.warmupViolation=true;
  inflight++; maxInflight=Math.max(maxInflight,inflight);
  let raw=''; for await(const chunk of req)raw+=chunk;
  const input=JSON.parse(raw);
  const evidence=input.messages.flatMap(message=>Array.isArray(message.content)?message.content:[])
    .filter(part=>part.type==='text'&&part.text.startsWith('=== PAGE')).map(part=>part.text).join('\n');
  const requestedPages=[...evidence.matchAll(/=== PAGE (\d+) \(text layer\)/g)].map(match=>Number(match[1]));
  let documents;
  if (evidence.includes('SINGLE_0_SOURCE') || evidence.includes('SINGLE_1_SOURCE')) {
    const title=evidence.includes('SINGLE_0_SOURCE')?'Single Delta':'Single Epsilon';
    documents=[{sourcePages:[1],answerPages:[],document:article(title,1,'TRUE','P1')}];
  } else {
    const selected=specs.filter(spec=>evidence.includes(spec.marker));
    assert(selected.length>0,'request must contain source evidence');
    if(childOnly)assert.deepEqual(requestedPages,[3,4],'child rerecognition must not include other articles');
    documents=selected.map(spec=>({sourcePages:spec.sourcePages,answerPages:spec.answerPages,
      testLabel:spec===specs[2]?'Test 2':'Test 1',document:article(spec.title,spec.number,spec.answer,spec.category)}));
  }
  report.requests.push({requestedPages,documentTitles:documents.map(d=>d.document.meta.title)});
  res.writeHead(200,{'Content-Type':'application/json'});
  const output = failNext ? {documents:[],warnings:['controlled malformed output']} : {documents,warnings:[]};
  failNext=false;
  await sleep(400); inflight--; completed++; report.maxInflight=maxInflight;
  res.end(JSON.stringify({choices:[{message:{content:JSON.stringify(output)},finish_reason:'stop'}],usage:{prompt_tokens:100,completion_tokens:100,total_tokens:200}}));
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
fs.writeFileSync(path.join(data,'config/llm-profiles.json'),JSON.stringify([{profileId:'multi-controlled',name:'Multi controlled',provider:'OpenAiCompatible',model:'multi-controlled',baseUrl:`http://127.0.0.1:${server.address().port}/v1`,enabled:true,forceJson:true,temperature:0,timeoutMs:30000}]));
fs.writeFileSync(path.join(data,'config/secrets/multi-controlled.key'),'local-test-token');
const dbRead=fn=>{const db=new DatabaseSync(path.join(data,'authoring_hub.db'),{readOnly:true});try{db.exec('PRAGMA busy_timeout=5000');return fn(db);}finally{db.close();}};
let session;
async function invoke(command,args){const result=await session.invoke(command,args);assert(result.ok,`${command}: ${result.error}`);return result.value;}
async function settled(expected=5){
  const deadline=Date.now()+60000;
  while(Date.now()<deadline){
    const jobs=dbRead(db=>db.prepare('SELECT id,stage,last_error_code FROM processing_jobs_v2').all());
    if(jobs.some(job=>job.stage==='failed'))throw new Error(JSON.stringify(jobs));
    if(jobs.length===expected&&jobs.every(job=>job.stage==='ready_for_review'))return jobs;
    await sleep(250);
  }
  throw new Error('cloud documents did not settle');
}
try {
  session=await launchTauriAppCdp({exePath,runDir,appEnv:{PDF2TEST_AUTOMATION_SOURCE_FILES:[multiFile,...singleFiles].join(path.delimiter)}});
  await session.waitFor(`!!document.querySelector('[data-testid="library-import"]')`,{timeoutMs:40000});
  await session.clickSelector('[data-testid="library-import"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-cloud-multi"]')`);
  await session.clickSelectorWhenStable('[data-testid="import-pick-files"]');
  await session.waitFor(`document.querySelectorAll('[data-testid="import-picked-files"] li').length===3`);
  await session.clickSelector('[data-testid="import-start"]');
  await settled();
  await session.waitFor(`document.querySelectorAll('[data-testid="library-row"]').length===5`,{timeoutMs:30000});
  assert.equal(await session.evaluate(`!!document.querySelector('[data-testid="import-split-plan"]')`),false);
  assert.equal(report.requests.length,3,'one cloud request for each selected source PDF');
  assert.notEqual(report.warmupViolation,true,'the first cloud request runs alone to warm the prompt cache');
  report.checks.push(`cache warm-up: first request ran alone; peak concurrency afterwards ${maxInflight}`);
  const rows=dbRead(db=>db.prepare('SELECT id,title,canonical_ds_json FROM library_items_v2 ORDER BY title').all());
  const beta=rows.find(row=>row.title==='Article Beta'); assert(beta);
  const alpha=rows.find(row=>row.title==='Article Alpha'); const gamma=rows.find(row=>row.title==='Article Gamma');
  assert.deepEqual(JSON.parse(alpha.canonical_ds_json).answerKey.q1.labels,['TRUE']);
  assert.deepEqual(JSON.parse(gamma.canonical_ds_json).answerKey.q1.labels,['NOT GIVEN']);
  report.checks.push('three source PDFs -> five independently stored documents; repeated q1 answers stay separate');
  report.libraryScreenshot=await session.screenshot('five-documents');
  for(const row of rows){
    await session.clickSelector(`[data-item-id="${row.id}"] .library-row-main`);
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-title"]')`,{timeoutMs:15000});
    const workspace=await invoke('get_workspace_item',{itemId:row.id});
    assert.equal(workspace.ds.audit.source,'cloud_direct');
    await session.clickSelector('[data-testid="workspace-mode-student"]');
    await session.waitFor(`!!document.querySelector('.v2-passage-pane')`,{timeoutMs:15000});
    const rendered=await session.evaluate(`document.querySelector('.v2-passage-pane').innerText`);
    assert(rendered.includes(row.title));
    if(row.id===beta.id)report.previewScreenshot=await session.screenshot('beta-independent-preview');
    await session.clickSelector('[data-testid="workspace-back"]');
    await session.waitFor(`!!document.querySelector('[data-testid="library-row"]')`);
  }
  report.checks.push('all five documents open independently in editor and student preview');
  await session.clickSelector(`[data-item-id="${beta.id}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('[data-testid="workspace-title"]')`);
  await session.waitFor(`document.querySelector('[data-testid="workspace-part-select"]')?.disabled===false`);
  await session.clickSelectorWhenStable('[data-testid="workspace-title"] span');
  await session.waitFor(`!!document.querySelector('[data-testid="workspace-title-input"]')`);
  await session.evaluate(`(()=>{const e=document.querySelector('[data-testid="workspace-title-input"]');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,'Edited Beta');e.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  await session.pressKey('Enter',{code:'Enter',windowsVirtualKeyCode:13});
  await session.waitFor(`document.querySelector('[data-testid="workspace-title"]')?.textContent?.includes('Edited Beta')`);
  await session.waitFor(`document.querySelector('[data-testid="workspace-save-state"]')?.textContent==='已保存'`,{timeoutMs:15000});
  await session.clickSelector('[data-testid="workspace-back"]');
  await session.waitFor(`!!document.querySelector('[data-testid="library-row"]')`);
  await session.clickSelector(`[data-item-id="${beta.id}"] .library-row-main`);
  await session.waitFor(`document.querySelector('[data-testid="workspace-title"]')?.textContent?.includes('Edited Beta')`);
  report.checks.push('child document edits save and survive reopening');
  childOnly=true;
  await invoke('retry_processing',{itemId:beta.id});
  await settled();
  assert.equal(dbRead(db=>db.prepare('SELECT count(*) AS n FROM library_items_v2').get()).n,5);
  assert.deepEqual(report.requests.at(-1).requestedPages,[3,4]);
  report.checks.push('child rerecognition sees only original pages 3/4 and creates no duplicate rows');
  failNext=true;
  await invoke('retry_processing',{itemId:beta.id});
  const failedDeadline=Date.now()+20000;
  let failed;
  while(Date.now()<failedDeadline){
    failed=dbRead(db=>db.prepare('SELECT stage,local_status,cloud_status FROM processing_jobs_v2 WHERE id=?').get(beta.id));
    if(failed.stage==='failed')break;
    await sleep(200);
  }
  assert.equal(failed.stage,'failed');assert.equal(failed.local_status,'not_run');assert.equal(failed.cloud_status,'failed');
  assert.equal(dbRead(db=>db.prepare('SELECT count(*) AS n FROM library_items_v2').get()).n,5);
  assert(dbRead(db=>db.prepare('SELECT canonical_ds_json FROM library_items_v2 WHERE id=?').get(beta.id)).canonical_ds_json);
  report.checks.push('cloud failure preserves the existing child and other four items without a local recognition fallback');
  await invoke('retry_processing',{itemId:beta.id});await settled();
  report.jobs=dbRead(db=>db.prepare('SELECT id,stage,local_status,cloud_status FROM processing_jobs_v2').all());
  await session.evaluate(`localStorage.setItem('ielts-author-studio.app-settings.v1',JSON.stringify({nasDestination:${JSON.stringify(path.join(runDir,'nas'))},cloudTokenBudget:300000}))`);
  await session.clickSelector('[data-testid="workspace-back"]');
  await session.waitFor(`!!document.querySelector('[data-testid="library-row"]')`);
  for(const row of rows){
    await session.clickSelector(`[data-item-id="${row.id}"] .library-row-main`);
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-publish"]')`);
    await session.clickSelector('[data-testid="workspace-publish"]');
    await session.waitFor(`!!document.querySelector('[data-publish-outcome]')`,{timeoutMs:20000});
    const outcome=await session.evaluate(`document.querySelector('[data-publish-outcome]').getAttribute('data-publish-outcome')`);
    assert.equal(outcome,'published',`export must be student-loadable: ${row.title}`);
    await session.clickSelector('[data-testid="workspace-back"]');
    await session.waitFor(`!!document.querySelector('[data-testid="library-row"]')`);
  }
  report.checks.push('all five independent documents publish through the actual UI as student-loadable packages');
  report.verdict='passed';
} catch(error){report.verdict='failed';report.error=String(error);process.exitCode=1;report.failureScreenshot=await session?.screenshot('failure').catch(()=>null);}
finally{if(session)report.appOutput=(await session.close()).appOutput;await new Promise(resolve=>server.close(resolve));fs.writeFileSync(path.join(runDir,'report.json'),JSON.stringify(report,null,2));console.log(path.join(runDir,'report.json'));}
