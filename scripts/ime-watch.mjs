// Records what the input box sees while scripts/ime-type.ps1 types through the real Windows IME.
//
//   node scripts/ime-watch.mjs arm     clear the box, start logging key and composition events
//   node scripts/ime-watch.mjs read    print the box, the last history rows and the event log

import { connect } from "./cdp.mjs";

const cdp = await connect();
if (process.argv[2] === "arm") {
  await cdp.eval(`(() => {
    window.__imeLog = [];
    const inp = document.getElementById('input');
    inp.value = '';
    if (!window.__imeArmed) {
      window.__imeArmed = true;
      for (const t of ['compositionstart', 'compositionend']) {
        inp.addEventListener(t, (e) => __imeLog.push(t + ':' + e.data), true);
      }
      inp.addEventListener('keydown', (e) => __imeLog.push(
        'keydown key=' + e.key + ' kc=' + e.keyCode + ' composing=' + e.isComposing + ' value=' + JSON.stringify(inp.value)), true);
    }
    inp.focus();
  })()`);
  console.log("armed");
} else {
  console.log("box:", JSON.stringify(await cdp.eval(`document.getElementById('input').value`)));
  const rows = await cdp.eval(`[...document.querySelectorAll('#history li')].slice(-5)
    .map(li => li.querySelector('.text').firstChild.textContent + ' [' + li.querySelector('.status')?.textContent + ']')`);
  console.log("history:", rows);
  console.log((await cdp.eval(`(window.__imeLog ?? []).join(String.fromCharCode(10))`)) || "(no events)");
}
cdp.close();
