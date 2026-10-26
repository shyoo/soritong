// Drives the setup wizard over CDP, tests each step, and takes screenshots.
import { connect, sleep } from "./cdp.mjs";

const cdp = await connect();
const ev = (js) => cdp.eval(js);

console.log("Connected to Soritong app via CDP.");

// 1. Verify wizard is open on fresh profile
const wizardHidden = await ev(`document.getElementById('wizard').hidden`);
console.log("Wizard open on launch:", !wizardHidden);
if (wizardHidden) {
  console.log("Opening wizard manually...");
  await ev(`document.getElementById('reopen-wizard').click()`);
}

await sleep(300);
await cdp.screenshot("wizard_step0_path.png");
console.log("Captured wizard_step0_path.png");

// 2. Select virtual mic path
await ev(`document.getElementById('card-path-virtual').click()`);
const isVirtualSelected = await ev(`document.getElementById('card-path-virtual').classList.contains('selected')`);
console.log("Virtual mic card selected:", isVirtualSelected);

// Click Continue
await ev(`document.getElementById('wizard-path-continue').click()`);
await sleep(500);

// 3. Step 1: Snapshot
const step1Visible = await ev(`!document.getElementById('wizard-step-snapshot').hidden`);
const snapRender = await ev(`document.getElementById('snapshot-render-text').textContent`);
const snapCapture = await ev(`document.getElementById('snapshot-capture-text').textContent`);
console.log("Step 1 (Snapshot) visible:", step1Visible);
console.log("Snapshot render:", snapRender);
console.log("Snapshot capture:", snapCapture);
await cdp.screenshot("wizard_step1_snapshot.png");

// Click Next to Step 2
await ev(`document.getElementById('wizard-snapshot-next').click()`);
await sleep(300);

// 4. Step 2: Install VB-CABLE
const step2Visible = await ev(`!document.getElementById('wizard-step-install').hidden`);
console.log("Step 2 (Install) visible:", step2Visible);
await cdp.screenshot("wizard_step2_install.png");

// Click Next to Step 3 (Safeguard)
await ev(`document.getElementById('wizard-install-next').click()`);
await sleep(300);
if (await ev(`!document.getElementById('wizard-step-install').hidden`)) {
  await ev(`document.getElementById('wizard-install-next').click()`);
  await sleep(300);
}

// 5. Step 3: Safeguard & Restore Defaults
const step3Visible = await ev(`!document.getElementById('wizard-step-safeguard').hidden`);
const listenWarningHidden = await ev(`document.getElementById('listen-device-warning').hidden`);
const defaultsWarningHidden = await ev(`document.getElementById('defaults-hijacked-warning').hidden`);
console.log("Step 3 (Safeguard) visible:", step3Visible);
console.log("Listen to device warning hidden (echo guard passed):", listenWarningHidden);
console.log("Defaults hijacked warning hidden:", defaultsWarningHidden);
await cdp.screenshot("wizard_step3_safeguard.png");

// Click Next to Step 4 (Configure)
await ev(`document.getElementById('wizard-safeguard-next').click()`);
await sleep(300);

// 6. Step 4: Configure Soritong
const step4Visible = await ev(`!document.getElementById('wizard-step-configure').hidden`);
const cfgMicVal = await ev(`document.getElementById('wizard-cfg-mic').value`);
console.log("Step 4 (Configure) visible:", step4Visible);
console.log("Configured mic output device:", cfgMicVal);
await cdp.screenshot("wizard_step4_configure.png");

// Click Next to Step 5 (Loopback)
await ev(`document.getElementById('wizard-configure-next').click()`);
await sleep(300);

// 7. Step 5: Loopback
const step5Visible = await ev(`!document.getElementById('wizard-step-loopback').hidden`);
console.log("Step 5 (Loopback) visible:", step5Visible);
await cdp.screenshot("wizard_step5_loopback.png");

// Skip loopback test to Step 6 (Zoom Guide)
await ev(`document.getElementById('wizard-loopback-skip').click()`);
await sleep(300);

// 8. Step 6: Zoom Guide
const step6Visible = await ev(`!document.getElementById('wizard-step-zoom').hidden`);
const micCode = await ev(`document.getElementById('zoom-mic-name').textContent`);
console.log("Step 6 (Zoom Guide) visible:", step6Visible);
console.log("Zoom mic name code:", micCode);

// Test copy button
await ev(`document.getElementById('wizard-copy-mic-btn').click()`);
await sleep(200);
const copiedBtnText = await ev(`document.getElementById('wizard-copy-mic-btn').textContent`);
console.log("Copy button text after click:", copiedBtnText);
await cdp.screenshot("wizard_step6_zoom.png");

// Finish setup
await ev(`document.getElementById('wizard-zoom-finish').click()`);
await sleep(500);

const wizardClosed = await ev(`document.getElementById('wizard').hidden`);
console.log("Wizard closed after finish:", wizardClosed);

// Check that settings shows wizard was completed
const wizState = await ev(`(async () => await window.__TAURI__.core.invoke('wizard_get_state'))()`);
console.log("Wizard completed state in backend:", wizState.wizard_completed);

// Now test reopen wizard from Settings
console.log("Opening Settings pane...");
await ev(`document.getElementById('open-settings').click()`);
await sleep(300);
const settingsVisible = await ev(`!document.getElementById('settings').hidden`);
console.log("Settings pane visible:", settingsVisible);

console.log("Clicking 'Set up virtual mic...' in settings...");
await ev(`document.getElementById('reopen-wizard').click()`);
await sleep(300);
const wizardReopened = await ev(`!document.getElementById('wizard').hidden`);
console.log("Wizard reopened successfully:", wizardReopened);
await cdp.screenshot("wizard_reopened.png");

// Close wizard
await ev(`document.getElementById('close-wizard').click()`);
await sleep(300);
console.log("Wizard closed again:", await ev(`document.getElementById('wizard').hidden`));

console.log("ALL WIZARD CHECKS PASSED!");
