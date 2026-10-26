/* A recording starts only from an acknowledged, explicit audio choice. */
(() => {
  'use strict';
  const choices = new Set(['microphone', 'microphone-and-system']);
  function create({ persist, changed }) {
    let choice = null, pending = false, error = '', revision = 0, request = 0;
    const snapshot = () => ({ choice, pending, error, ready: choices.has(choice) && !pending && !error });
    const notify = () => changed(snapshot());
    return Object.freeze({
      snapshot,
      beginLoad: () => ({ revision, request: ++request }),
      load(value, token) {
        if (pending || error || token.revision !== revision || token.request !== request) return;
        choice = choices.has(value) ? value : null;
        error = '';
        notify();
      },
      async save(value) {
        if (!choices.has(value) || pending) return;
        pending = true;
        error = '';
        revision++;
        notify();
        try {
          await persist(value);
          choice = value;
        } catch (cause) {
          error = `Audio choice could not be saved. Choose again before recording. ${String(cause?.message || cause || '')}`;
        } finally {
          pending = false;
          revision++;
          notify();
        }
      },
      startArgs({ detected = false } = {}) {
        if (!snapshot().ready) throw new Error(error || 'Choose microphone only or microphone + system audio before recording.');
        return choice === 'microphone'
          ? { intent: 'room', source: 'manual' }
          : { intent: 'call', source: detected ? 'call_detect' : 'manual', allowDegraded: false };
      },
    });
  }
  window.MinutesRecordingAudio = Object.freeze({ create });
})();
