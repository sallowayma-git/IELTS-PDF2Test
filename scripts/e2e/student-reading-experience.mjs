// Evidence: real browser component/CSS interaction only. This does not replace Tauri UI acceptance.
import path from 'node:path';
import fs from 'node:fs';
import { ensureVite, launchChrome, createPage, evaluate, navigate, screenshot, setViewport, waitFor, repoRoot } from '../lib/cdp.mjs';
const vite = await ensureVite();
const chrome = await launchChrome();
let cdp;
const out = path.join(repoRoot, 'output/playwright/student-reading');
const results = [];
const run = (fn, arg) => evaluate(cdp, '(' + fn.toString() + ')(' + JSON.stringify(arg) + ')');
const assert = (ok, message) => { if (!ok) throw new Error(message); };
try {
  ({ cdp } = await createPage(chrome.port));
  await setViewport(cdp, { width: 1440, height: 960 });
  await navigate(cdp, 'http://127.0.0.1:1420/scripts/e2e/fixtures/student-reading-experience.html');
  await waitFor('heading inspector', () => run(() => !!document.querySelector('[aria-label="标题字号"]')));
  await run(() => { const select=document.querySelector('[aria-label="标题字号"]'); select.value='1.3'; select.dispatchEvent(new Event('change',{bubbles:true})); });
  await waitFor('heading scale', () => run(() => document.querySelector('.v2-heading')?.style.getPropertyValue('--node-font-scale')==='1.3'));
  await screenshot(cdp, path.join(out, 'author-font-inspector.png'));
  await run(() => document.querySelector('#toggle-mode').click());
  await waitFor('student canvas', () => run(() => !!document.querySelector('.is-student')));
  for (const [size, expected] of [['normal',16],['large',18],['xlarge',20]]) {
    await run((size) => document.querySelector('[aria-label="Text size '+size+'"]').click(), size);
    const actual = await run(() => {
      const font = (selector) => parseFloat(getComputedStyle(document.querySelector(selector)).fontSize);
      const heading=document.querySelector('.v2-heading');
      return {passage:font('.v2-passage-pane'),question:font('.v2-question-pane'),option:font('.v2-choice-item'),heading:font('.v2-heading'),gap:parseFloat(getComputedStyle(heading).marginBottom),label:!!document.querySelector('.v2-editor-task-badge')};
    });
    assert(actual.passage===expected && actual.question===expected && actual.option===expected, 'both panes/options must zoom: '+JSON.stringify(actual));
    assert(Math.abs(actual.heading-expected*1.15*1.3)<.1 && actual.gap>=actual.heading*.8-.1 && !actual.label, 'heading/label: '+JSON.stringify(actual));
    results.push({size,...actual});
  }
  for (const selector of ['.v2-passage-pane .v2-text','.v2-question-pane .v2-text']) {
    await run((selector) => {
      const node=document.querySelector(selector).firstChild;
      const range=document.createRange();range.setStart(node,0);range.setEnd(node,Math.min(8,node.length));
      window.getSelection().removeAllRanges();window.getSelection().addRange(range);document.dispatchEvent(new Event('selectionchange'));
    },selector);
    await waitFor('highlight toolbar',()=>run(()=>!!document.querySelector('.student-highlight-toolbar')));
    await run(()=>document.querySelector('.student-highlight-toolbar button').click());
    await waitFor('highlight rendered',()=>run((selector)=>!!document.querySelector(selector.split(' ')[0]+' .hl'),selector));
  }
  const choice=await run(()=>{const el=document.querySelector('input[type="checkbox"]');el.click();return el.checked;});
  assert(choice,'answer checkbox must work after highlighting');
  await screenshot(cdp,path.join(out,'student-zoom-highlight.png'));
  await run(()=>document.querySelector('.hl').click());
  await waitFor('remove toolbar',()=>run(()=>!!document.querySelector('.student-highlight-toolbar')));
  await run(()=>[...document.querySelectorAll('.student-highlight-toolbar button')].find((button)=>button.textContent==='Remove highlight').click());
  assert(await run(()=>document.querySelectorAll('.hl').length)===1,'remove must unwrap one pane highlight');
  fs.mkdirSync(out,{recursive:true});
  fs.writeFileSync(path.join(out,'report.json'),JSON.stringify({evidence:'browser component/CSS, not Tauri',results,highlightBothPanes:true,answerControl:true},null,2));
  console.log(JSON.stringify({status:'pass',out,results}));
} finally { cdp?.close();chrome.close();vite.close(); }
