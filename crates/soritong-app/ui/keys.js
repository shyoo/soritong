// What a key press in the message box means. No DOM access, so it runs under `node --test`.
//
// Korean IME rule: while a syllable is being composed, never submit, clear or edit the text.
// The Windows Microsoft IME sends Enter as keyCode 229 / key "Process" (code "Enter") while
// composing; it commits the syllable, fires compositionend, and only sometimes sends a second,
// ordinary Enter keydown. So Enter during composition marks a submit as pending, and the submit
// happens after compositionend, from the second keydown if one comes or from a deferred check
// if not. Either way it reads the box only after the last syllable has landed in it.

function isEnter(ev) {
  return (
    ev.key === "Enter" || ev.code === "Enter" || ev.code === "NumpadEnter" || ev.keyCode === 13
  );
}

export function isComposingKey(ev, composing) {
  return Boolean(composing || ev.isComposing || ev.keyCode === 229 || ev.key === "Process");
}

/**
 * @param {{key?: string, code?: string, keyCode?: number, isComposing?: boolean,
 *          shiftKey?: boolean, altKey?: boolean, ctrlKey?: boolean, metaKey?: boolean}} ev
 * @param {{composing: boolean, empty: boolean}} state
 * @returns {"submit" | "submit-after-composition" | "newline" | "stop" | "recall" | "none"}
 */
export function decideKey(ev, { composing, empty }) {
  if (isComposingKey(ev, composing)) {
    if (isEnter(ev) && !ev.shiftKey && !ev.altKey && !ev.metaKey) return "submit-after-composition";
    return "none";
  }
  if (isEnter(ev)) {
    if (ev.shiftKey) return "newline";
    if (ev.altKey || ev.metaKey) return "none";
    return "submit";
  }
  if (ev.key === "Escape") return "stop";
  if (ev.key === "ArrowUp" && empty && !ev.shiftKey && !ev.altKey && !ev.ctrlKey && !ev.metaKey) {
    return "recall";
  }
  return "none";
}

/**
 * Tracks composition and turns key events into calls. Wire `keydown`, `compositionstart` and
 * `compositionend` to it; `onKeyDown` returns true when the caller should preventDefault.
 *
 * @param {{submit: () => void, stop: () => void, recall: () => void,
 *          defer?: (fn: () => void) => void}} actions
 */
export function createInputController({ submit, stop, recall, defer = (fn) => setTimeout(fn, 30) }) {
  let composing = false;
  let pending = false;

  const flush = () => {
    if (pending && !composing) {
      pending = false;
      submit();
    }
  };

  return {
    get composing() {
      return composing;
    },
    compositionStart() {
      // A new syllable began before the deferred submit ran: the box now holds text the user
      // did not mean to send, so drop the submit rather than send half a word.
      composing = true;
      pending = false;
    },
    compositionEnd() {
      composing = false;
      if (pending) defer(flush);
    },
    /** @returns {boolean} preventDefault */
    onKeyDown(ev, { empty }) {
      switch (decideKey(ev, { composing, empty })) {
        case "submit-after-composition":
          pending = true;
          return false; // let the IME commit the syllable
        case "submit":
          pending = false;
          submit();
          return true;
        case "stop":
          stop();
          return true;
        case "recall":
          recall();
          return true;
        default:
          return false;
      }
    },
  };
}
