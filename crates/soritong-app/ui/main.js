// The page: renders state from Rust and calls its commands. No logic that matters lives here
// except the key handling, which is in keys.js so it can be tested.

import { createInputController } from "./keys.js";
import { t } from "./strings.js";
import {
  createWizardController,
  formatRms,
  formatDelay,
  isCableName,
  STEP,
  TOTAL_STEPS,
} from "./wizard.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const el = (tag, props = {}, ...children) => {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...children);
  return e;
};

const MAX_HISTORY = 100;

const state = {
  engines: [],
  engine: null,
  voices: [],
  devices: [],
  audio: null,
  phrases: [],
  editingPhrases: false,
  lastSent: "",
  modelReady: true,
  history: new Map(), // id -> { li, text, engine, voice }
};

const input = $("input");

// ---- static labels ----

function applyStrings() {
  for (const n of document.querySelectorAll("[data-t]")) n.textContent = t(n.dataset.t);
  for (const n of document.querySelectorAll("[data-title]")) {
    n.title = t(n.dataset.title);
    n.setAttribute("aria-label", n.title);
  }
  for (const n of document.querySelectorAll("[data-placeholder]")) n.placeholder = t(n.dataset.placeholder);
}

function showError(msg) {
  const li = el("li", {}, el("span", { className: "text" }, el("span", { className: "error", textContent: msg })));
  $("history").append(li);
  li.scrollIntoView({ block: "end" });
}

const call = (cmd, args) =>
  invoke(cmd, args).catch((e) => {
    showError(String(e));
    throw e;
  });

// ---- engine and voice ----

const currentEngine = () => state.engines.find((e) => e.id === state.engine);

function renderEngines() {
  const sel = $("engine");
  sel.replaceChildren(
    ...state.engines.map((e) => el("option", { value: e.id, textContent: e.name + (e.offline ? " 🔒" : "") })),
  );
  sel.value = state.engine ?? "";
}

function renderBadges() {
  const e = currentEngine();
  const v = state.voices.find((v) => v.id === $("voice").value);
  const badges = [];
  if (e?.offline) badges.push(t("offline"));
  if (e?.needs_model_download) badges.push(t("needsModel"));
  if (v?.kind === "cloned") badges.push("🧬 " + t("cloned"));
  $("badges").replaceChildren(...badges.map((b) => el("span", { className: "badge", textContent: b })));
}

async function loadVoices() {
  const sel = $("voice");
  state.voices = [];
  sel.replaceChildren();
  if (currentEngine()?.needs_model_download && !state.modelReady) {
    sel.append(el("option", { value: "", textContent: t("noVoices") }));
    renderBadges();
    return;
  }
  try {
    const { voices, selected } = await invoke("list_voices", { engine: state.engine });
    state.voices = voices;
    sel.replaceChildren(
      ...voices.map((v) =>
        el("option", { value: v.id, textContent: v.name + (v.kind === "cloned" ? " 🧬" : "") }),
      ),
    );
    if (!voices.length) sel.append(el("option", { value: "", textContent: t("noVoices") }));
    sel.value = selected ?? "";
  } catch (e) {
    sel.append(el("option", { value: "", textContent: t("noVoices") }));
    showError(String(e));
  }
  renderBadges();
}

async function updateModelStatus() {
  const e = currentEngine();
  if (e?.needs_model_download) {
    state.modelReady = await invoke("check_model_ready", { engine: state.engine }).catch(() => false);
  } else {
    state.modelReady = true;
  }
  const showDownload = Boolean(e?.needs_model_download && !state.modelReady);
  $("download-panel").hidden = !showDownload;
  if (!showDownload) {
    $("download-progress-container").hidden = true;
    $("download-status").textContent = "";
  }
}

$("engine").addEventListener("change", async (ev) => {
  state.engine = ev.target.value;
  await updateModelStatus();
  await loadVoices();
  call("set_selection", { engine: state.engine, voice: $("voice").value || null });
});

$("voice").addEventListener("change", () => {
  renderBadges();
  call("set_selection", { engine: state.engine, voice: $("voice").value || null });
});

// ---- devices ----

function deviceOptions(selectedId, withDefaultHint) {
  const opts = [
    el("option", {
      value: "",
      textContent: t(withDefaultHint ? "systemDefaultOption" : "systemDefault"),
    }),
    ...state.devices.map((d) =>
      el("option", { value: d.id, textContent: d.name + (d.virtual_mic ? ` (${d.virtual_mic.family})` : "") }),
    ),
  ];
  if (selectedId && !state.devices.some((d) => d.id === selectedId)) {
    opts.push(el("option", { value: selectedId, textContent: `${selectedId} (?)` }));
  }
  return opts;
}

function renderDevices() {
  const a = state.audio;
  $("mic").replaceChildren(...deviceOptions(a.mic_device, true));
  $("mic").value = a.mic_device ?? "";
  $("monitor-device").replaceChildren(...deviceOptions(a.monitor_device, false));
  $("monitor-device").value = a.monitor_device ?? "";
  $("mic-volume").value = a.mic_volume;
  $("monitor-volume").value = a.monitor_volume;
  $("monitor-toggle").setAttribute("aria-pressed", String(a.monitor_enabled));
  $("monitor-panel").hidden = !a.monitor_enabled;
  renderMicHint();
}

function renderMicHint() {
  const d = state.devices.find((d) => d.id === state.audio.mic_device);
  $("mic-hint").textContent = d?.virtual_mic
    ? t("zoomPickMic", { name: d.virtual_mic.zoom_mic_name })
    : state.audio.mic_device
      ? ""
      : t("systemDefaultHint");
}

async function loadDevices() {
  try {
    state.devices = await invoke("list_outputs");
  } catch (e) {
    state.devices = [];
    showError(String(e));
  }
  renderDevices();
}

function saveAudio() {
  const a = state.audio;
  a.mic_device = $("mic").value || null;
  a.monitor_device = $("monitor-device").value || null;
  a.mic_volume = Number($("mic-volume").value);
  a.monitor_volume = Number($("monitor-volume").value);
  renderMicHint();
  call("set_audio", { audio: a });
}

$("refresh").addEventListener("click", loadDevices);
// `change`, not `input`: reconfiguring may drop queued audio, so not on every slider tick.
for (const id of ["mic", "monitor-device", "mic-volume", "monitor-volume"]) {
  $(id).addEventListener("change", saveAudio);
}
$("monitor-toggle").addEventListener("click", () => {
  state.audio.monitor_enabled = !state.audio.monitor_enabled;
  renderDevices();
  saveAudio();
});

// ---- speaking and history ----

const STATUS = { queued: "queued", started: "speaking", finished: "done", cancelled: "cancelled", failed: "failed" };

async function speak(text) {
  if (currentEngine()?.needs_model_download && !state.modelReady) {
    showError(t("downloadDisabledSpeaking"));
    return;
  }
  const voice = $("voice").value;
  if (!text.trim()) return;
  state.lastSent = text;
  try {
    await invoke("speak", { text, engine: state.engine, voice });
  } catch (e) {
    showError(String(e));
  }
}

function addHistory({ id, text, engine, voice }) {
  const status = el("span", { className: "status queued", textContent: t("queued") });
  const replay = el("button", { textContent: "▶", title: t("replay") });
  const edit = el("button", { textContent: "✎", title: t("edit") });
  const body = el("span", { className: "text", textContent: text });
  const li = el("li", {}, body, status, replay, edit);
  replay.addEventListener("click", () => invoke("speak", { text, engine, voice }).catch((e) => showError(String(e))));
  edit.addEventListener("click", () => {
    input.value = text;
    input.focus();
  });
  state.history.set(id, { li, status, body });
  const list = $("history");
  list.append(li);
  while (list.children.length > MAX_HISTORY) list.firstElementChild.remove();
  li.scrollIntoView({ block: "end" });
}

function onSpeakEvent(ev) {
  if (ev.type === "queued") return addHistory(ev);
  const h = state.history.get(ev.id);
  if (!h) return;
  const s = STATUS[ev.type];
  h.status.className = `status ${s}`;
  h.status.textContent = t(s);
  if (ev.type === "failed") {
    h.body.querySelector(".error")?.remove();
    h.body.append(el("span", { className: "error", textContent: ev.message }));
  }
  if (ev.type !== "started") state.history.delete(ev.id); // terminal; a replay gets a new id
}

const stop = () => invoke("stop");
$("stop").addEventListener("click", () => {
  stop();
  input.focus();
});

// ---- the input box ----

const keys = createInputController({
  submit: () => {
    const text = input.value;
    if (!text.trim()) return;
    input.value = "";
    speak(text);
  },
  stop,
  recall: () => {
    input.value = state.lastSent;
    input.setSelectionRange(input.value.length, input.value.length);
  },
});

input.addEventListener("compositionstart", () => keys.compositionStart());
input.addEventListener("compositionend", () => keys.compositionEnd());
input.addEventListener("keydown", (ev) => {
  if (keys.onKeyDown(ev, { empty: input.value === "" })) ev.preventDefault();
});

// ---- quick phrases ----

function renderPhrases() {
  const chips = state.phrases.map((p, i) => {
    const chip = el("button", { className: "chip", textContent: p });
    if (state.editingPhrases) {
      const x = el("span", { className: "x", textContent: "×", title: t("removePhrase") });
      chip.append(x);
      chip.addEventListener("click", () => {
        state.phrases.splice(i, 1);
        savePhrases();
      });
    } else {
      chip.addEventListener("click", () => {
        speak(p);
        input.focus();
      });
    }
    return chip;
  });
  if (state.editingPhrases) {
    const add = el("input", { type: "text", placeholder: t("addPhrase") });
    add.addEventListener("keydown", (ev) => {
      // Same IME rule as the main box: Enter that commits a syllable is not "add".
      if (ev.key !== "Enter" || ev.isComposing || ev.keyCode === 229) return;
      ev.preventDefault();
      if (add.value.trim()) {
        state.phrases.push(add.value.trim());
        savePhrases().then(() => $("chips").querySelector("input")?.focus());
      }
    });
    chips.push(add);
  }
  $("chips").replaceChildren(...chips);
  const btn = $("edit-phrases");
  btn.textContent = state.editingPhrases ? "✓" : "✎";
  btn.title = t(state.editingPhrases ? "donePhrases" : "editPhrases");
}

async function savePhrases() {
  renderPhrases();
  await call("set_quick_phrases", { phrases: state.phrases });
}

$("edit-phrases").addEventListener("click", () => {
  state.editingPhrases = !state.editingPhrases;
  renderPhrases();
  if (state.editingPhrases) $("chips").querySelector("input")?.focus();
});

// ---- window ----

$("pin").addEventListener("click", async () => {
  const on = $("pin").getAttribute("aria-pressed") !== "true";
  await call("set_always_on_top", { on });
  $("pin").setAttribute("aria-pressed", String(on));
});

// ---- settings pane ----

function fieldInput(engine, f) {
  if (f.kind === "secret") {
    const box = el("input", {
      type: "password",
      autocomplete: "off",
      placeholder: engine.secrets_set[f.key] ? t("secretSet") : t("secretUnset"),
    });
    box.dataset.secret = f.key;
    const clear = el("button", { type: "button", textContent: t("clearSecret") });
    clear.disabled = !engine.secrets_set[f.key];
    clear.addEventListener("click", () => saveEngine(engine.id, { clear_secrets: [f.key] }));
    return el("div", { className: "field-row" }, box, clear);
  }
  let box;
  if (f.kind === "select") {
    box = el("select", {}, el("option", { value: "", textContent: f.default ? `(${f.default})` : "—" }));
    box.append(...f.options.map((o) => el("option", { value: o, textContent: o })));
  } else {
    box = el("input", { type: f.kind === "number" ? "number" : "text", placeholder: f.default ?? "" });
    if (f.kind === "number") box.step = "any";
  }
  box.value = engine.values[f.key] ?? "";
  box.dataset.value = f.key;
  return box;
}

function renderEngineSettings() {
  const sets = state.engines.map((e) => {
    const legend = [e.name, e.offline ? t("offline") : "", e.needs_model_download ? `(${t("needsModel")})` : ""]
      .filter(Boolean)
      .join(" ");
    const fs = el("fieldset", {}, el("legend", { textContent: legend }));
    if (!e.fields.length) {
      fs.append(el("p", { className: "hint", textContent: t("noFields") }));
      return fs;
    }
    for (const f of e.fields) {
      fs.append(el("label", {}, el("span", { textContent: f.label }), fieldInput(e, f)));
      if (f.help) fs.append(el("p", { className: "hint", textContent: f.help }));
    }
    const status = el("span", { className: "status" });
    const save = el("button", { type: "button", textContent: t("save") });
    save.addEventListener("click", () => {
      const values = {};
      const secrets = {};
      for (const n of fs.querySelectorAll("[data-value]")) values[n.dataset.value] = n.value;
      for (const n of fs.querySelectorAll("[data-secret]")) if (n.value) secrets[n.dataset.secret] = n.value;
      saveEngine(e.id, { values, secrets });
    });
    fs.append(el("div", { className: "field-row" }, save, status));
    fs.dataset.engine = e.id;
    return fs;
  });
  $("engine-settings").replaceChildren(...sets);
}

async function saveEngine(id, update) {
  try {
    const view = await invoke("save_engine_settings", { engine: id, update });
    state.engines = state.engines.map((e) => (e.id === id ? view : e));
    renderEngineSettings(); // also drops typed secrets from the DOM
    const status = document.querySelector(`fieldset[data-engine="${id}"] .status`);
    if (status) status.textContent = t("saved");
    if (id === state.engine) loadVoices();
  } catch (e) {
    const status = document.querySelector(`fieldset[data-engine="${id}"] .status`);
    if (status) {
      status.textContent = String(e);
      status.classList.add("bad");
    }
  }
}

function openSettings() {
  renderEngineSettings();
  $("settings").hidden = false;
}

function closeSettings() {
  $("settings").hidden = true;
  renderEngineSettings(); // forget any secret typed but not saved
  input.focus();
}

$("open-settings").addEventListener("click", openSettings);
$("close-settings").addEventListener("click", closeSettings);
$("save-shortcut").addEventListener("click", async () => {
  const status = $("shortcut-status");
  try {
    await invoke("set_shortcut", { shortcut: $("shortcut").value });
    status.textContent = t("saved");
    status.classList.remove("bad");
  } catch (e) {
    status.textContent = String(e);
    status.classList.add("bad");
  }
});

// ---- wizard controller and UI ----

let wizardController = null;
let selectedPathChoice = "virtual_mic";

function updateWizardProgress(step) {
  $("wizard-step-indicator").textContent = t("stepIndicator", { current: step + 1, total: TOTAL_STEPS });
  const dots = document.querySelectorAll("#wizard-progress .progress-dot");
  dots.forEach((dot) => {
    const s = Number(dot.dataset.step);
    dot.classList.toggle("active", s === step);
    dot.classList.toggle("done", s < step);
  });
  const lines = document.querySelectorAll("#wizard-progress .progress-line");
  lines.forEach((line, idx) => {
    line.classList.toggle("done", idx < step);
  });
}

function renderWizardStep(step, wState, extra = {}) {
  updateWizardProgress(step);

  const stepPanes = [
    $("wizard-step-path"),
    $("wizard-step-snapshot"),
    $("wizard-step-install"),
    $("wizard-step-safeguard"),
    $("wizard-step-configure"),
    $("wizard-step-loopback"),
    $("wizard-step-zoom"),
  ];

  stepPanes.forEach((pane, idx) => {
    if (pane) pane.hidden = idx !== step;
  });

  if (step === STEP.PATH) {
    $("card-path-virtual").classList.toggle("selected", selectedPathChoice === "virtual_mic");
    $("card-path-direct").classList.toggle("selected", selectedPathChoice === "direct");
  } else if (step === STEP.SNAPSHOT) {
    const snap = wState?.snapshot ?? wState?.current_defaults;
    $("snapshot-render-text").textContent = t("snapshotRender", {
      name: snap?.render_console?.friendly_name ?? "—",
    });
    $("snapshot-capture-text").textContent = t("snapshotCapture", {
      name: snap?.capture_console?.friendly_name ?? "—",
    });
  } else if (step === STEP.INSTALL) {
    const installed = Boolean(wState?.cable_installed);
    $("cable-already-installed").hidden = !installed;
    $("cable-needs-install").hidden = installed;
    const installBtn = $("wizard-install-btn");
    installBtn.disabled = extra.installing;
    if (extra.installing) {
      $("wizard-install-status").textContent = t("installingCable");
    } else if (extra.cableNotDetectedNotice) {
      $("wizard-install-status").textContent = t("cableNotInstalled");
    } else {
      $("wizard-install-status").textContent = "";
    }
  } else if (step === STEP.SAFEGUARD) {
    const listenWarning = Boolean(wState?.listen_to_device_enabled);
    $("listen-device-warning").hidden = !listenWarning;
    $("listen-device-ok").hidden = listenWarning;

    const renderName = wState?.current_defaults?.render_console?.friendly_name;
    const captureName = wState?.current_defaults?.capture_console?.friendly_name;
    const hijacked = Boolean(wState?.defaults_are_cable || isCableName(renderName) || isCableName(captureName));

    $("defaults-hijacked-warning").hidden = !hijacked;
    $("defaults-ok").hidden = hijacked;
  } else if (step === STEP.CONFIGURE) {
    $("wizard-cfg-mic").value =
      wState?.cable_render_name ?? "CABLE Input (VB-Audio Virtual Cable)";

    const monitorSelect = $("wizard-cfg-monitor-device");
    const outputOpts = (wState?.real_speakers?.length ? wState.real_speakers : state.devices).map((d) =>
      el("option", {
        value: d.id,
        textContent: d.name,
      }),
    );
    monitorSelect.replaceChildren(...outputOpts);
    if (wState?.real_speakers?.[0]?.id) {
      monitorSelect.value = wState.real_speakers[0].id;
    }
  } else if (step === STEP.LOOPBACK) {
    const res = extra.loopbackResult;
    const nextBtn = $("wizard-loopback-next");
    const status = $("wizard-loopback-status");
    const details = $("wizard-loopback-details");
    const meterBar = $("wizard-loopback-meter-bar");

    if (!res) {
      nextBtn.disabled = true;
      meterBar.style.width = "0%";
      status.textContent = "";
      details.textContent = "";
    } else if (res.success) {
      nextBtn.disabled = false;
      const rmsVal = res.rms || 0.1;
      const pct = Math.min(100, Math.round(rmsVal * 120));
      meterBar.style.width = `${pct}%`;
      status.textContent = t("loopbackPass");
      status.className = "status status-ok";
      details.textContent = t("loopbackStats", {
        delay: formatDelay(res.delay_ms),
        rms: formatRms(res.rms),
        peak: formatRms(res.peak),
      });
    } else {
      nextBtn.disabled = true;
      meterBar.style.width = "0%";
      status.textContent = t("loopbackFail");
      status.className = "status bad";
      details.textContent = res.message ?? "";
    }
  } else if (step === STEP.ZOOM) {
    $("zoom-mic-name").textContent =
      wState?.zoom_mic_name ?? "CABLE Output (VB-Audio Virtual Cable)";
  }
}

function openWizard(initialStep = null) {
  $("wizard").hidden = false;
  wizardController.init(initialStep);
}

function closeWizard() {
  $("wizard").hidden = true;
  input.focus();
}

function setupWizardController() {
  wizardController = createWizardController({
    invoke,
    copyToClipboard: async (text) => {
      try {
        if (navigator?.clipboard?.writeText) {
          await navigator.clipboard.writeText(text);
          return true;
        }
      } catch (_) {}
      return false;
    },
    onStepChange: (step, wState, extra) => {
      renderWizardStep(step, wState, extra);
    },
    onComplete: async () => {
      closeWizard();
      await loadDevices();
      input.focus();
    },
    onClose: () => {
      closeWizard();
    },
    showError,
  });

  // Step 0: Path Choice
  $("card-path-virtual").addEventListener("click", () => {
    selectedPathChoice = "virtual_mic";
    $("card-path-virtual").classList.add("selected");
    $("card-path-direct").classList.remove("selected");
  });
  $("card-path-direct").addEventListener("click", () => {
    selectedPathChoice = "direct";
    $("card-path-direct").classList.add("selected");
    $("card-path-virtual").classList.remove("selected");
  });
  $("wizard-path-continue").addEventListener("click", () => {
    wizardController.selectPath(selectedPathChoice);
  });
  $("wizard-path-cancel").addEventListener("click", closeWizard);

  // Step 1: Snapshot
  $("wizard-snapshot-back").addEventListener("click", () => {
    wizardController.setStep(STEP.PATH);
  });
  $("wizard-snapshot-next").addEventListener("click", () => {
    wizardController.setStep(STEP.INSTALL);
  });

  // Step 2: Install VB-CABLE
  $("wizard-install-back").addEventListener("click", () => {
    wizardController.setStep(STEP.SNAPSHOT);
  });
  $("wizard-install-btn").addEventListener("click", async () => {
    await wizardController.installCable();
  });
  $("wizard-manual-link").addEventListener("click", (e) => {
    e.preventDefault();
    invoke("wizard_open_vbcable_page").catch((err) => alert(String(err)));
  });
  $("wizard-reboot-btn").addEventListener("click", async () => {
    await wizardController.setRebootPending(true);
  });
  $("wizard-install-next").addEventListener("click", async () => {
    if (wizardController.state?.cable_installed) {
      wizardController.setStep(STEP.SAFEGUARD);
    } else {
      await wizardController.checkCableInstalled();
      if ($("wizard-install-next").dataset.checkedOnce === "true") {
        wizardController.setStep(STEP.SAFEGUARD);
      } else {
        $("wizard-install-next").dataset.checkedOnce = "true";
      }
    }
  });

  // Step 3: Safeguard & Restore Defaults
  $("wizard-safeguard-back").addEventListener("click", () => {
    wizardController.setStep(STEP.INSTALL);
  });
  $("wizard-restore-btn").addEventListener("click", async () => {
    $("restore-status").textContent = "...";
    await wizardController.restoreDefaults();
    $("restore-status").textContent = t("restoreDone");
  });
  $("wizard-safeguard-next").addEventListener("click", () => {
    wizardController.setStep(STEP.CONFIGURE);
  });

  // Step 4: Configure Soritong
  $("wizard-configure-back").addEventListener("click", () => {
    wizardController.setStep(STEP.SAFEGUARD);
  });
  $("wizard-cfg-monitor-enable").addEventListener("change", (e) => {
    $("wizard-cfg-monitor-select-label").hidden = !e.target.checked;
  });
  $("wizard-configure-next").addEventListener("click", async () => {
    const monitorEnabled = $("wizard-cfg-monitor-enable").checked;
    const monitorDevice = $("wizard-cfg-monitor-device").value || null;
    await wizardController.configureSoritong({
      monitorEnabled,
      monitorDevice,
    });
  });

  // Step 5: Loopback Test
  $("wizard-loopback-back").addEventListener("click", () => {
    wizardController.setStep(STEP.CONFIGURE);
  });
  $("wizard-loopback-btn").addEventListener("click", async () => {
    $("wizard-loopback-btn").disabled = true;
    $("wizard-loopback-status").textContent = t("testingLoopback");
    $("wizard-loopback-status").className = "status";
    $("wizard-loopback-meter-bar").style.width = "40%";
    await wizardController.runLoopback();
    $("wizard-loopback-btn").disabled = false;
  });
  $("wizard-loopback-skip").addEventListener("click", () => {
    wizardController.setStep(STEP.ZOOM);
  });
  $("wizard-loopback-next").addEventListener("click", () => {
    wizardController.setStep(STEP.ZOOM);
  });

  // Step 6: Zoom Guide
  $("wizard-zoom-back").addEventListener("click", () => {
    wizardController.setStep(STEP.LOOPBACK);
  });
  $("wizard-copy-mic-btn").addEventListener("click", async () => {
    const name = $("zoom-mic-name").textContent;
    const ok = await wizardController.copyMicName(name);
    if (ok) {
      const orig = $("wizard-copy-mic-btn").textContent;
      $("wizard-copy-mic-btn").textContent = t("copied");
      setTimeout(() => {
        $("wizard-copy-mic-btn").textContent = orig;
      }, 2000);
    }
  });
  $("wizard-zoom-finish").addEventListener("click", async () => {
    await wizardController.complete();
  });

  // Top header close button
  $("close-wizard").addEventListener("click", closeWizard);

  // Settings pane reopen button
  $("reopen-wizard").addEventListener("click", () => {
    closeSettings();
    openWizard(STEP.PATH);
  });
}

document.addEventListener("keydown", (ev) => {
  if (ev.key !== "Escape" || ev.isComposing || ev.keyCode === 229) return;
  if (!$("wizard").hidden) closeWizard();
  else if (!$("settings").hidden) closeSettings();
  else if (ev.target !== input) stop();
});

// ---- start ----

async function init() {
  applyStrings();
  setupWizardController();

  await listen("speak-event", (e) => onSpeakEvent(e.payload));
  await listen("focus-input", () => input.focus());

  const s = await invoke("get_state");
  state.engines = s.engines;
  state.engine = s.engine;
  state.phrases = s.quick_phrases;
  state.audio = {
    mic_device: s.mic_device,
    mic_volume: s.mic_volume,
    monitor_enabled: s.monitor_enabled,
    monitor_device: s.monitor_device,
    monitor_volume: s.monitor_volume,
  };
  $("pin").setAttribute("aria-pressed", String(s.always_on_top));
  $("shortcut").value = s.shortcut;
  $("settings-path").textContent = s.settings_path ? t("settingsFile", { path: s.settings_path }) : "";

  state.modelReady = s.model_ready;

  $("download-btn").addEventListener("click", async () => {
    $("download-btn").disabled = true;
    $("download-status").textContent = t("downloadingModel");
    $("download-status").classList.remove("bad");
    $("download-progress-container").hidden = false;
    $("download-progress").value = 0;
    $("download-percent").textContent = "0%";
    try {
      await invoke("download_model", { engine: state.engine });
      state.modelReady = true;
      $("download-panel").hidden = true;
      $("download-btn").disabled = false;
      $("download-progress-container").hidden = true;
      await loadVoices();
    } catch (err) {
      $("download-status").textContent = String(err);
      $("download-status").classList.add("bad");
      $("download-btn").disabled = false;
      showError(String(err));
    }
  });

  await listen("model-download-progress", (e) => {
    const p = e.payload;
    $("download-progress-container").hidden = false;
    $("download-progress").value = Math.round(p.percent);
    const mbDown = (p.downloaded_bytes / (1024 * 1024)).toFixed(1);
    const mbTotal = (p.total_bytes / (1024 * 1024)).toFixed(1);
    $("download-percent").textContent = `${Math.round(p.percent)}% (${mbDown}/${mbTotal} MB)`;
  });

  renderEngines();
  await updateModelStatus();
  renderPhrases();
  await Promise.all([loadVoices(), loadDevices()]);

  // Check wizard status: if not completed or reboot pending, launch wizard automatically
  const wizState = await invoke("wizard_get_state").catch(() => null);
  if (wizState && (!wizState.completed || wizState.reboot_pending)) {
    openWizard();
  } else {
    input.focus();
  }
}

init().catch((e) => showError(String(e)));
