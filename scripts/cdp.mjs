// Minimal Chrome DevTools Protocol client for the app's WebView2 page. No dependencies (Node ≥ 22
// has a global WebSocket). Start the app with remote debugging on: scripts/run-app.ps1.
//
//   import { connect } from "./cdp.mjs";
//   const cdp = await connect();               // port from SORITONG_CDP_PORT, default 9333
//   await cdp.eval("document.title");          // multi-statement code: wrap in (() => { ... })()
//   await cdp.send("Input.insertText", { text: "안녕" });
//   await cdp.screenshot("shot.png");
//   cdp.close();

import { writeFileSync } from "node:fs";

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export async function connect(port = Number(process.env.SORITONG_CDP_PORT ?? 9333)) {
  let targets;
  for (let i = 0; ; i++) {
    try {
      targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
      break;
    } catch {
      if (i >= 20) throw new Error(`no CDP endpoint on :${port}; was the app started via scripts/run-app.ps1?`);
      await sleep(500);
    }
  }
  const page = targets.find((t) => t.type === "page");
  if (!page) throw new Error("CDP is up but there is no page yet");
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.addEventListener("open", resolve);
    ws.addEventListener("error", reject);
  });
  let next = 0;
  const waiting = new Map();
  ws.addEventListener("message", (m) => {
    const d = JSON.parse(m.data);
    if (d.id && waiting.has(d.id)) {
      waiting.get(d.id)(d);
      waiting.delete(d.id);
    }
  });
  const send = (method, params = {}) =>
    new Promise((resolve) => {
      const id = ++next;
      waiting.set(id, resolve);
      ws.send(JSON.stringify({ id, method, params }));
    });
  return {
    send,
    /** Evaluates in the page and returns the value; throws with the page's exception. */
    async eval(expression) {
      const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
      const ex = r.result?.exceptionDetails;
      if (ex) throw new Error(ex.exception?.description ?? ex.text);
      return r.result?.result?.value;
    },
    /** A trusted key press, as the page's keydown handlers see it. */
    async key(key, { code = key, keyCode, shift = false } = {}) {
      const base = { key, code, windowsVirtualKeyCode: keyCode, modifiers: shift ? 8 : 0 };
      await send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...base });
      await send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
    },
    async screenshot(path) {
      const r = await send("Page.captureScreenshot", { format: "png" });
      writeFileSync(path, Buffer.from(r.result.data, "base64"));
    },
    close: () => ws.close(),
  };
}
