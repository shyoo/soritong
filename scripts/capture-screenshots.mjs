// Retakes the README screenshots from the real app: the wizard's first step, then the main window
// after three messages spoken with the Edge voices into CABLE Input (monitor off, so nothing is
// heard). Needs fresh settings, so that the wizard opens, and VB-CABLE installed.
//
//   Remove-Item -Recurse $env:TEMP\soritong-shots -ErrorAction Ignore
//   scripts/run-app.ps1 -SettingsDir $env:TEMP\soritong-shots
//   node scripts/capture-screenshots.mjs
//   scripts/run-app.ps1 -Stop

import { connect, sleep } from "./cdp.mjs";

const cdp = await connect();
const ev = (js) => cdp.eval(js);
await sleep(1500);

if (!(await ev(`!document.getElementById('wizard').hidden`))) {
  console.error("The wizard is not open: start the app with fresh settings (see the header).");
  process.exit(1);
}
await cdp.screenshot("docs/screenshots/wizard.png");

await ev(`(() => {
  [...document.querySelectorAll('#wizard button')]
    .find((b) => b.offsetParent && /Cancel/.test(b.textContent))?.click();
  const set = (id, v) => {
    const s = document.getElementById(id);
    s.value = v;
    s.dispatchEvent(new Event('change', { bubbles: true }));
  };
  set('engine', 'edge');
  set('mic', 'CABLE Input (VB-Audio Virtual Cable)');
})()`);
await sleep(3000);

const lastRow = () => ev(`[...document.querySelectorAll('#history li')].at(-1)?.textContent ?? ''`);
const type = (text) => ev(`document.getElementById('input').focus()`).then(() => cdp.send("Input.insertText", { text }));
for (const m of ["안녕하세요, 늦은 시간에 다들 반가워요.", "저는 이번 책에서 3장이 제일 좋았어요.", "I agree with that point."]) {
  await type(m);
  await cdp.key("Enter", { keyCode: 13 });
  for (let i = 0; i < 60 && !/done/.test(await lastRow()); i++) await sleep(250);
}
await type("다음 모임은 언제인가요?");
await sleep(500);
await cdp.screenshot("docs/screenshots/main-window.png");
console.log("saved docs/screenshots/wizard.png and main-window.png");
process.exit(0);
