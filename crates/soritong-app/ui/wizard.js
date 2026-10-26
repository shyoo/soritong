// Wizard state machine and logic.
// Pure functions are exported for unit testing in ui-tests/wizard.test.mjs.

export const STEP = {
  PATH: 0,
  SNAPSHOT: 1,
  INSTALL: 2,
  SAFEGUARD: 3,
  CONFIGURE: 4,
  LOOPBACK: 5,
  ZOOM: 6,
};

export const TOTAL_STEPS = 7;

/**
 * Converts a linear RMS amplitude [0.0, 1.0] to dBFS string.
 * @param {number} rms
 * @returns {string} e.g. "-18.5" or "-inf"
 */
export function formatRms(rms) {
  if (!rms || rms <= 1e-5) return "-inf";
  const db = 20 * Math.log10(rms);
  return db.toFixed(1);
}

/**
 * Formats milliseconds delay.
 * @param {number|null|undefined} ms
 * @returns {string}
 */
export function formatDelay(ms) {
  if (ms == null) return "—";
  return String(Math.round(ms));
}

/**
 * Checks whether an audio device name is VB-CABLE.
 * @param {string|null|undefined} name
 * @returns {boolean}
 */
export function isCableName(name) {
  if (!name) return false;
  const lower = name.toLowerCase();
  return lower.includes("vb-audio") || lower.includes("cable input") || lower.includes("cable output");
}

/**
 * Determines which step to display based on backend state.
 * @param {object} state
 * @returns {number}
 */
export function decideInitialStep(state) {
  if (!state) return STEP.PATH;
  const path = state.wizard_path ?? state.path;
  if (!path || path === "direct_share" || path === "direct") return STEP.PATH;

  // If user rebooted or needs driver:
  const reboot = state.reboot_pending ?? state.wizard_reboot_pending;
  if (reboot || !state.cable_installed) {
    return STEP.INSTALL;
  }

  // If CABLE hijacked default devices or listen-to-device is active:
  const renderName = state.snapshot?.render_console?.friendly_name ?? state.current_defaults?.render_console?.friendly_name ?? state.default_render?.name;
  const captureName = state.snapshot?.capture_console?.friendly_name ?? state.current_defaults?.capture_console?.friendly_name ?? state.default_capture?.name;
  const renderIsCable = isCableName(renderName);
  const captureIsCable = isCableName(captureName);
  if (state.listen_to_device_enabled || state.defaults_are_cable || renderIsCable || captureIsCable) {
    return STEP.SAFEGUARD;
  }

  // If Soritong's output device is not yet configured to CABLE:
  if (!state.soritong_configured) {
    return STEP.CONFIGURE;
  }

  // If loopback hasn't been run or passed:
  if (!state.loopback_passed) {
    return STEP.LOOPBACK;
  }

  return STEP.ZOOM;
}

/**
 * Creates the wizard controller.
 */
export function createWizardController({
  invoke,
  copyToClipboard = (text) => navigator?.clipboard?.writeText(text),
  onStepChange = () => {},
  onComplete = () => {},
  onClose = () => {},
  showError = (err) => console.error(err),
}) {
  let currentStep = STEP.PATH;
  let wizardState = null;
  let loopbackResult = null;
  let installing = false;

  async function loadState() {
    try {
      wizardState = await invoke("wizard_get_state");
      return wizardState;
    } catch (e) {
      showError(String(e));
      return null;
    }
  }

  function setStep(step) {
    if (step < 0) step = 0;
    if (step >= TOTAL_STEPS) step = TOTAL_STEPS - 1;
    currentStep = step;
    onStepChange(currentStep, wizardState, { loopbackResult, installing });
  }

  return {
    get currentStep() {
      return currentStep;
    },
    get state() {
      return wizardState;
    },
    get loopbackResult() {
      return loopbackResult;
    },
    get installing() {
      return installing;
    },

    async init(forceStep = null) {
      await loadState();
      if (forceStep !== null) {
        setStep(forceStep);
      } else {
        setStep(decideInitialStep(wizardState));
      }
    },

    setStep,

    async selectPath(path) {
      try {
        const backendPath = path === "direct" ? "direct_share" : path;
        await invoke("wizard_choose_path", { path: backendPath });
        if (wizardState) wizardState.wizard_path = backendPath;
        if (backendPath === "direct_share") {
          await invoke("wizard_complete");
          if (wizardState) wizardState.wizard_completed = true;
          onComplete({ path: "direct_share" });
        } else {
          // Virtual mic path: snapshot defaults next
          await this.takeSnapshot();
          setStep(STEP.SNAPSHOT);
        }
      } catch (e) {
        showError(String(e));
      }
    },

    async takeSnapshot() {
      try {
        const snap = await invoke("wizard_take_snapshot");
        if (wizardState) {
          wizardState.snapshot = snap;
          wizardState.has_snapshot = true;
        }
        return snap;
      } catch (e) {
        showError(String(e));
        return null;
      }
    },

    async installCable() {
      installing = true;
      onStepChange(currentStep, wizardState, { loopbackResult, installing });
      try {
        const res = await invoke("wizard_install_vbcable");
        return res;
      } catch (e) {
        showError(String(e));
        return null;
      } finally {
        installing = false;
        await loadState();
        onStepChange(currentStep, wizardState, { loopbackResult, installing });
      }
    },

    async setRebootPending(pending = true) {
      try {
        await invoke("wizard_set_reboot_pending", { pending });
        if (wizardState) wizardState.reboot_pending = pending;
        onClose();
      } catch (e) {
        showError(String(e));
      }
    },

    async checkCableInstalled() {
      await loadState();
      if (wizardState?.cable_installed) {
        setStep(STEP.SAFEGUARD);
      } else {
        onStepChange(currentStep, wizardState, {
          loopbackResult,
          installing,
          cableNotDetectedNotice: true,
        });
      }
    },

    async restoreDefaults(renderId = null, captureId = null) {
      try {
        const report = await invoke("wizard_restore_defaults", {
          renderId: renderId ?? wizardState?.snapshot?.render_console?.endpoint_id ?? null,
          captureId: captureId ?? wizardState?.snapshot?.capture_console?.endpoint_id ?? null,
        });
        await loadState();
        onStepChange(currentStep, wizardState, { loopbackResult, installing });
        return report;
      } catch (e) {
        showError(String(e));
        return null;
      }
    },

    async configureSoritong({ micDevice, monitorEnabled, monitorDevice, monitorVolume = 1.0 }) {
      try {
        await invoke("wizard_configure_soritong", {
          micDevice: micDevice ?? wizardState?.cable_input_id ?? null,
          monitorEnabled,
          monitorDevice: monitorDevice ?? wizardState?.real_speakers?.[0]?.id ?? null,
          monitorVolume,
        });
        if (wizardState) wizardState.soritong_configured = true;
        setStep(STEP.LOOPBACK);
      } catch (e) {
        showError(String(e));
      }
    },

    async runLoopback() {
      try {
        loopbackResult = await invoke("wizard_run_loopback");
        if (loopbackResult?.success && wizardState) {
          wizardState.loopback_passed = true;
        }
        onStepChange(currentStep, wizardState, { loopbackResult, installing });
        return loopbackResult;
      } catch (e) {
        loopbackResult = {
          success: false,
          rms: 0,
          peak: 0,
          delay_ms: null,
          message: String(e),
        };
        onStepChange(currentStep, wizardState, { loopbackResult, installing });
        return loopbackResult;
      }
    },

    async copyMicName(name = "CABLE Output (VB-Audio Virtual Cable)") {
      try {
        await copyToClipboard(name);
        return true;
      } catch (e) {
        showError(String(e));
        return false;
      }
    },

    async complete() {
      try {
        await invoke("wizard_complete");
        if (wizardState) wizardState.wizard_completed = true;
        onComplete({ path: "virtual_mic" });
      } catch (e) {
        showError(String(e));
      }
    },

    close() {
      onClose();
    },
  };
}
