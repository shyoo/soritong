// Drives a running app (started with scripts/run-app.ps1) through the basics and reports what it
// saw: an English message going queued → speaking → done, a simulated IME composition and
// Shift+Enter. The IME part uses CDP imeSetComposition, which produces Chromium's composition
// events but not the real Windows IME. For the real one, use scripts/ime-type.ps1.
//
//   node scripts/app-smoke.mjs [screenshot.png]      exit code 1 if any check fails

import { connect, sleep } from "./cdp.mjs";

const cdp = await connect();
const ev = (js) => cdp.eval(js);
let failed = false;
const check = (ok, what) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  failed ||= !ok;
};

console.log("selects:", await ev(`JSON.stringify([...document.querySelectorAll('select')].map(s => s.id + '=' + s.value))`));
console.log("quick phrases:", await ev(`[...document.querySelectorAll('.chip')].map(c => c.textContent).join(' · ')`));

// Every status a history row passes through, including the one it is created with.
await ev(`(() => {
  window.__statusLog = [];
  const note = (li) => {
    const s = li.querySelector?.('.status');
    if (s) __statusLog.push(li.querySelector('.text').firstChild.textContent + ' → ' + s.textContent);
  };
  new MutationObserver((ms) => {
    for (const m of ms) {
      m.addedNodes.forEach(note);
      const li = (m.target.nodeType === 1 ? m.target : m.target.parentElement).closest('li');
      if (li) note(li);
    }
  }).observe(document.getElementById('history'), { subtree: true, childList: true, characterData: true });
  const i = document.getElementById('input');
  i.value = '';
  i.focus();
})()`);

const history = () => ev(`[...document.querySelectorAll('#history li .text')].map(t => t.firstChild.textContent)`);
const box = () => ev(`document.getElementById('input').value`);

await cdp.send("Input.insertText", { text: "I agree with that point." });
await cdp.key("Enter", { keyCode: 13 });
for (let i = 0; i < 40; i++) {
  await sleep(200);
  const cur = await ev(`[...new Set(__statusLog)]`);
  if (["queued", "speaking", "done"].every((s) => cur.some((l) => l.endsWith(s)))) break;
}
const log = await ev(`[...new Set(__statusLog)]`);
console.log("transitions:", log);
check(["queued", "speaking", "done"].every((s) => log.some((l) => l.endsWith(s))), "message went queued → speaking → done");
check((await box()) === "", "box cleared after sending");

// Microsoft-IME-like: last syllable composing, Enter arrives as keyCode 229, commit, maybe a plain Enter.
async function compose(prefix, last, secondEnter) {
  await cdp.send("Input.insertText", { text: prefix });
  await cdp.send("Input.imeSetComposition", { text: last, selectionStart: 1, selectionEnd: 1 });
  await cdp.key("Process", { code: "Enter", keyCode: 229 });
  await cdp.send("Input.insertText", { text: last }); // commits: compositionend
  if (secondEnter) await cdp.key("Enter", { keyCode: 13 });
  await sleep(300);
}
const before = (await history()).length;
await compose("안녕하세", "요", true);
await compose("감사합니", "다", false);
await compose("네 맞아", "요", true);
const spoken = (await history()).slice(before);
check(JSON.stringify(spoken) === JSON.stringify(["안녕하세요", "감사합니다", "네 맞아요"]), `simulated IME spoke ${JSON.stringify(spoken)}`);
check((await box()) === "", "no syllable leaked into the box");

await cdp.send("Input.insertText", { text: "첫 줄" });
await cdp.key("Enter", { keyCode: 13, shift: true });
await sleep(200);
// CDP's rawKeyDown inserts no text, so only "not sent, box kept" is checkable here. The newline
// itself was seen with real keys (ime-type.ps1 "{SHIFTENTER}").
check((await history()).length === before + 3 && (await box()) === "첫 줄", "Shift+Enter does not send or clear");
await ev(`document.getElementById('input').value = ''`);

if (process.argv[2]) {
  await cdp.screenshot(process.argv[2]);
  console.log("screenshot:", process.argv[2]);
}
cdp.close();
process.exit(failed ? 1 : 0);
