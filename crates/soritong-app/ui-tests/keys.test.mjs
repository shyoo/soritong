// Run: node --test crates/soritong-app/ui-tests/
import { test } from "node:test";
import assert from "node:assert/strict";
import { createInputController, decideKey } from "../ui/keys.js";

const idle = { composing: false, empty: false };

test("plain Enter submits, Shift+Enter is a newline, Esc stops", () => {
  assert.equal(decideKey({ key: "Enter", code: "Enter", keyCode: 13 }, idle), "submit");
  assert.equal(decideKey({ key: "Enter", keyCode: 13, shiftKey: true }, idle), "newline");
  assert.equal(decideKey({ key: "Escape", keyCode: 27 }, idle), "stop");
  assert.equal(decideKey({ key: "a", keyCode: 65 }, idle), "none");
});

test("never submits while composing, however the browser reports it", () => {
  const composingEnter = [
    { key: "Process", code: "Enter", keyCode: 229, isComposing: true },
    { key: "Process", code: "Enter", keyCode: 229 }, // isComposing missing
    { key: "Enter", code: "Enter", keyCode: 13, isComposing: true },
  ];
  for (const ev of composingEnter) {
    assert.equal(decideKey(ev, idle), "submit-after-composition", JSON.stringify(ev));
  }
  // Our own compositionstart flag counts even if the event claims otherwise.
  assert.equal(
    decideKey({ key: "Enter", keyCode: 13 }, { composing: true, empty: false }),
    "submit-after-composition",
  );
  // Esc and Up during composition belong to the IME.
  assert.equal(decideKey({ key: "Escape", keyCode: 229, isComposing: true }, idle), "none");
  assert.equal(decideKey({ key: "ArrowUp", keyCode: 229 }, { composing: true, empty: true }), "none");
  // Shift+Enter while composing commits and then inserts a newline: never a submit.
  assert.equal(decideKey({ key: "Process", code: "Enter", keyCode: 229, shiftKey: true }, idle), "none");
});

test("Up recalls only in an empty box", () => {
  assert.equal(decideKey({ key: "ArrowUp" }, { composing: false, empty: true }), "recall");
  assert.equal(decideKey({ key: "ArrowUp" }, idle), "none");
});

/** A fake text box driven by the controller, with a manual deferred queue. */
function harness() {
  const box = { value: "" };
  const sent = [];
  const deferred = [];
  const ctl = createInputController({
    submit: () => {
      if (box.value.trim()) sent.push(box.value);
      box.value = "";
    },
    stop: () => sent.push("<stop>"),
    recall: () => (box.value = sent.at(-1) ?? ""),
    defer: (fn) => deferred.push(fn),
  });
  const key = (ev) => ctl.onKeyDown(ev, { empty: box.value === "" });
  const runDeferred = () => deferred.splice(0).forEach((fn) => fn());
  return { box, sent, ctl, key, runDeferred };
}

const imeEnter = { key: "Process", code: "Enter", keyCode: 229, isComposing: true };
const enter = { key: "Enter", code: "Enter", keyCode: 13, isComposing: false };

test("MS IME: Enter commits the last syllable, then a second Enter keydown submits once", () => {
  const h = harness();
  h.ctl.compositionStart();
  h.box.value = "안녕하세"; // "요" still composing
  assert.equal(h.key(imeEnter), false, "must not preventDefault the committing Enter");
  assert.deepEqual(h.sent, []);
  h.box.value = "안녕하세요"; // IME commits
  h.ctl.compositionEnd();
  assert.equal(h.key(enter), true);
  h.runDeferred();
  assert.deepEqual(h.sent, ["안녕하세요"]);
  assert.equal(h.box.value, "");
});

test("MS IME: Enter commits and no second keydown comes; the deferred check submits", () => {
  const h = harness();
  h.ctl.compositionStart();
  h.key(imeEnter);
  h.box.value = "감사합니다";
  h.ctl.compositionEnd();
  assert.deepEqual(h.sent, [], "not before the deferred check");
  h.runDeferred();
  assert.deepEqual(h.sent, ["감사합니다"]);
  // Nothing leaks into the next message.
  h.ctl.compositionStart();
  h.box.value = "네";
  h.ctl.compositionEnd();
  h.runDeferred();
  assert.deepEqual(h.sent, ["감사합니다"]);
  assert.equal(h.box.value, "네");
});

test("a new composition before the deferred check cancels that submit", () => {
  const h = harness();
  h.ctl.compositionStart();
  h.key(imeEnter);
  h.box.value = "좋아요";
  h.ctl.compositionEnd();
  h.ctl.compositionStart(); // user already typing the next syllable
  h.box.value = "좋아요ㄴ";
  h.runDeferred();
  h.ctl.compositionEnd();
  h.runDeferred();
  assert.deepEqual(h.sent, []);
});

test("Shift+Enter while composing never submits", () => {
  const h = harness();
  h.ctl.compositionStart();
  h.box.value = "첫 줄";
  h.key({ ...imeEnter, shiftKey: true });
  h.ctl.compositionEnd();
  h.key({ ...enter, shiftKey: true });
  h.runDeferred();
  assert.deepEqual(h.sent, []);
});

test("English: Enter submits, Up in the empty box recalls, Esc stops", () => {
  const h = harness();
  h.box.value = "I agree";
  assert.equal(h.key(enter), true);
  assert.deepEqual(h.sent, ["I agree"]);
  assert.equal(h.key({ key: "ArrowUp" }), true);
  assert.equal(h.box.value, "I agree");
  assert.equal(h.key({ key: "ArrowUp" }), false, "Up with text moves the caret as usual");
  h.key({ key: "Escape" });
  assert.equal(h.sent.at(-1), "<stop>");
});
