// Captures documentation screenshots using CDP:
// 1. Wizard Step 1: Virtual Microphone Setup
// 2. Main Window: Clean chat bubbles with Korean and English samples
//
// Usage:
//   scripts/run-app.ps1
//   node scripts/capture-screenshots.mjs
//   scripts/run-app.ps1 -Stop

import { connect, sleep } from "./cdp.mjs";
import { resolve } from "node:path";

const cdp = await connect();
const ev = (js) => cdp.eval(js);

console.log("Connected to CDP");

const shotDir = resolve("docs/screenshots");
const wizardShot = resolve(shotDir, "wizard-step-1.png");
const mainShot = resolve(shotDir, "main-window.png");

// 1. Open and capture Wizard Step 1
console.log("Opening wizard for step 1 screenshot...");
await ev(`(() => {
  const reopen = document.getElementById('reopen-wizard');
  if (reopen) reopen.click();
  const w = document.getElementById('wizard');
  if (w) w.hidden = false;
})()`);
await sleep(1000);

console.log("Capturing wizard step 1 screenshot...");
await cdp.screenshot(wizardShot);
console.log("Saved", wizardShot);

// 2. Close wizard
await ev(`(() => {
  const close = document.getElementById('close-wizard') || document.getElementById('wizard-path-cancel');
  if (close) close.click();
  const w = document.getElementById('wizard');
  if (w) w.hidden = true;
})()`);
await sleep(600);

// 3. Populate sample chat bubbles for main window screenshot
await ev(`(() => {
  const history = document.getElementById('history');
  if (history) {
    history.innerHTML = '';
    const samples = [
      { text: "안녕하세요, 오늘 회의 시작하겠습니다.", status: "done", time: "10:00" },
      { text: "네, 공유해 주신 화면 잘 보입니다.", status: "done", time: "10:02" },
      { text: "I agree with that point. Let us proceed with this plan.", status: "done", time: "10:05" },
      { text: "다음 안건으로 넘어가겠습니다.", status: "speaking", time: "10:07" }
    ];
    for (const s of samples) {
      const li = document.createElement('li');
      li.className = 'history-item ' + s.status;
      li.innerHTML = '<span class="text">' + s.text + '</span><span class="meta"><span class="status">' + s.status + '</span><span class="time">' + s.time + '</span></span>';
      history.appendChild(li);
    }
  }
  const input = document.getElementById('input');
  if (input) {
    input.value = '질문 있으신 분 계신가요?';
  }
})()`);
await sleep(500);

console.log("Capturing main window screenshot...");
await cdp.screenshot(mainShot);
console.log("Saved", mainShot);

cdp.close();
console.log("Screenshots captured successfully.");
