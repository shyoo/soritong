// Run: node --test crates/soritong-app/ui-tests/wizard.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  formatRms,
  formatDelay,
  isCableName,
  decideInitialStep,
  createWizardController,
  STEP,
  TOTAL_STEPS,
} from "../ui/wizard.js";

test("formatRms converts linear amplitude to dBFS", () => {
  assert.equal(formatRms(1.0), "0.0");
  assert.equal(formatRms(0.5), "-6.0");
  assert.equal(formatRms(0.1), "-20.0");
  assert.equal(formatRms(0.0), "-inf");
  assert.equal(formatRms(1e-6), "-inf");
  assert.equal(formatRms(null), "-inf");
  assert.equal(formatRms(undefined), "-inf");
});

test("formatDelay formats milliseconds cleanly", () => {
  assert.equal(formatDelay(0), "0");
  assert.equal(formatDelay(15.4), "15");
  assert.equal(formatDelay(25), "25");
  assert.equal(formatDelay(null), "—");
  assert.equal(formatDelay(undefined), "—");
});

test("isCableName identifies VB-CABLE endpoints", () => {
  assert.equal(isCableName("CABLE Input (VB-Audio Virtual Cable)"), true);
  assert.equal(isCableName("CABLE Output (VB-Audio Virtual Cable)"), true);
  assert.equal(isCableName("Speakers (Realtek Audio)"), false);
  assert.equal(isCableName("Headphones"), false);
  assert.equal(isCableName(null), false);
  assert.equal(isCableName(""), false);
});

test("decideInitialStep selects the right starting step", () => {
  // Fresh install with no path chosen:
  assert.equal(decideInitialStep(null), STEP.PATH);
  assert.equal(decideInitialStep({ path: null }), STEP.PATH);
  assert.equal(decideInitialStep({ path: "direct" }), STEP.PATH);

  // Virtual mic chosen, reboot pending:
  assert.equal(
    decideInitialStep({ path: "virtual_mic", reboot_pending: true, cable_installed: false }),
    STEP.INSTALL,
  );

  // Virtual mic chosen, cable not installed:
  assert.equal(
    decideInitialStep({ path: "virtual_mic", reboot_pending: false, cable_installed: false }),
    STEP.INSTALL,
  );

  // Cable installed, but listen-to-device is enabled (echo danger):
  assert.equal(
    decideInitialStep({
      path: "virtual_mic",
      cable_installed: true,
      listen_to_device_enabled: true,
    }),
    STEP.SAFEGUARD,
  );

  // Cable installed, but default render was hijacked:
  assert.equal(
    decideInitialStep({
      path: "virtual_mic",
      cable_installed: true,
      listen_to_device_enabled: false,
      default_render: { name: "CABLE Input (VB-Audio Virtual Cable)" },
    }),
    STEP.SAFEGUARD,
  );

  // Cable installed, safeguard ok, not yet configured in Soritong:
  assert.equal(
    decideInitialStep({
      path: "virtual_mic",
      cable_installed: true,
      listen_to_device_enabled: false,
      default_render: { name: "Speakers (Realtek)" },
      soritong_configured: false,
    }),
    STEP.CONFIGURE,
  );

  // Configured, but loopback not tested:
  assert.equal(
    decideInitialStep({
      path: "virtual_mic",
      cable_installed: true,
      listen_to_device_enabled: false,
      default_render: { name: "Speakers (Realtek)" },
      soritong_configured: true,
      loopback_passed: false,
    }),
    STEP.LOOPBACK,
  );

  // Ready for Zoom instructions:
  assert.equal(
    decideInitialStep({
      path: "virtual_mic",
      cable_installed: true,
      listen_to_device_enabled: false,
      default_render: { name: "Speakers (Realtek)" },
      soritong_configured: true,
      loopback_passed: true,
    }),
    STEP.ZOOM,
  );
});

test("createWizardController step navigation and actions", async () => {
  const commandsCalled = [];
  const fakeState = {
    completed: false,
    path: null,
    reboot_pending: false,
    cable_installed: false,
    default_render: { id: "real_spk", name: "Realtek Speakers" },
    default_capture: { id: "real_mic", name: "Realtek Mic" },
    listen_to_device_enabled: false,
    cable_render_id: "cable_in_id",
    cable_render_name: "CABLE Input",
    cable_capture_id: "cable_out_id",
    cable_capture_name: "CABLE Output",
    suggested_monitor_device: "real_spk",
    cable_url: "https://example.com/vbcable.zip",
    cable_page_url: "https://example.com",
    cable_sha256: "abc",
  };

  const stepsVisited = [];
  let completedData = null;
  let copiedText = "";

  const controller = createWizardController({
    invoke: async (cmd, args) => {
      commandsCalled.push({ cmd, args });
      if (cmd === "wizard_get_state") return { ...fakeState };
      if (cmd === "wizard_choose_path") {
        fakeState.path = args.path;
        return null;
      }
      if (cmd === "wizard_take_snapshot") {
        return {
          render: fakeState.default_render,
          capture: fakeState.default_capture,
        };
      }
      if (cmd === "wizard_install_vbcable") {
        fakeState.cable_installed = true;
        return { launched: true, reboot_suggested: true, manual_url: "https://example.com" };
      }
      if (cmd === "wizard_restore_defaults") {
        return { render_restored: true, capture_restored: false, restored_devices: [] };
      }
      if (cmd === "wizard_configure_soritong") {
        return null;
      }
      if (cmd === "wizard_run_loopback") {
        return {
          success: true,
          rms: 0.15,
          peak: 0.7,
          delay_ms: 18,
          message: "ok",
        };
      }
      if (cmd === "wizard_complete") {
        fakeState.completed = true;
        return null;
      }
      return null;
    },
    copyToClipboard: async (text) => {
      copiedText = text;
    },
    onStepChange: (step) => {
      stepsVisited.push(step);
    },
    onComplete: (data) => {
      completedData = data;
    },
  });

  await controller.init();
  assert.equal(controller.currentStep, STEP.PATH);

  // Choosing direct completes immediately
  await controller.selectPath("direct");
  assert.deepEqual(completedData, { path: "direct_share" });

  // Reset and test virtual mic path
  fakeState.completed = false;
  fakeState.path = null;
  completedData = null;
  await controller.selectPath("virtual_mic");
  assert.equal(controller.currentStep, STEP.SNAPSHOT);

  // Next to install
  controller.setStep(STEP.INSTALL);
  assert.equal(controller.currentStep, STEP.INSTALL);

  // Run install
  await controller.installCable();
  assert.equal(controller.state.cable_installed, true);

  // Go to safeguard
  controller.setStep(STEP.SAFEGUARD);
  await controller.restoreDefaults();

  // Configure Soritong
  await controller.configureSoritong({
    micDevice: "cable_in_id",
    monitorEnabled: true,
    monitorDevice: "real_spk",
    monitorVolume: 1.0,
  });
  assert.equal(controller.currentStep, STEP.LOOPBACK);

  // Run loopback
  const res = await controller.runLoopback();
  assert.equal(res.success, true);
  assert.equal(res.delay_ms, 18);

  // Move to Zoom instructions
  controller.setStep(STEP.ZOOM);
  assert.equal(controller.currentStep, STEP.ZOOM);

  // Copy mic name
  await controller.copyMicName("CABLE Output (VB-Audio Virtual Cable)");
  assert.equal(copiedText, "CABLE Output (VB-Audio Virtual Cable)");

  // Complete setup
  await controller.complete();
  assert.deepEqual(completedData, { path: "virtual_mic" });
});
