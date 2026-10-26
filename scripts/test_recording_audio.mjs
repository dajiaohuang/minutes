import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import test from 'node:test';

const script = readFileSync(new URL('../tauri/src/scripts/recording-audio.js', import.meta.url), 'utf8');
function create(persist = async () => {}) {
  const window = {};
  vm.runInNewContext(script, { window });
  return window.MinutesRecordingAudio.create({ persist, changed: () => {} });
}
function load(controller, value) { controller.load(value, controller.beginLoad()); }
function args(controller, options) { return JSON.parse(JSON.stringify(controller.startArgs(options))); }
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test('idle renders preserve a native picker selection until its change event commits', () => {
  const html = readFileSync(new URL('../tauri/src/index.html', import.meta.url), 'utf8');
  const render = html.match(/    function renderMeetingAudioChoice\(\) \{[\s\S]*?\n    \}/)?.[0];
  assert(render);
  let value = '', disabled = false, valueWrites = 0, disabledWrites = 0;
  const picker = {
    get value() { return value; },
    set value(next) { valueWrites++; value = next; },
    get disabled() { return disabled; },
    set disabled(next) { disabledWrites++; disabled = next; },
  };
  const state = { choice: null, pending: false, error: '', ready: false };
  const context = vm.createContext({
    recordingAudioChoice: { snapshot: () => state },
    meetingAudioSource: picker, meetingAudioHelp: null,
    document: { getElementById: () => null },
    footer: { classList: { contains: () => false } },
    syncPrimaryCaptureLabel: () => {},
  });
  vm.runInContext(`const renderedMeetingAudioChoices = new WeakMap(); ${render}`, context);
  const paint = () => vm.runInContext('renderMeetingAudioChoice()', context);
  paint();

  // A native popup can update its value before WebKit dispatches change.
  // Simulate that user action without counting it as an application write.
  value = 'microphone-and-system';
  paint(); paint(); paint();
  assert.equal(value, 'microphone-and-system');
  assert.equal(valueWrites, 0);
  assert.equal(disabledWrites, 0);

  state.pending = true;
  paint(); paint();
  assert.equal(value, 'microphone-and-system');
  assert.equal(disabled, true);
  assert.equal(disabledWrites, 1);

  Object.assign(state, { choice: 'microphone-and-system', pending: false, ready: true });
  paint(); paint();
  assert.equal(value, 'microphone-and-system');
  assert.equal(disabled, false);
  assert.equal(valueWrites, 0);
  assert.equal(disabledWrites, 2);

  // A rejected save restores the acknowledged value, allowing the same
  // failed option to emit another change event when the user retries it.
  value = 'microphone';
  state.pending = true;
  paint();
  Object.assign(state, { pending: false, error: 'Audio choice could not be saved.', ready: false });
  paint(); paint();
  assert.equal(value, 'microphone-and-system');
  assert.equal(valueWrites, 1);
  value = 'microphone';
  Object.assign(state, { pending: true, error: '' });
  paint();
  assert.equal(value, 'microphone');
});

test('fresh or unknown preferences cannot start a microphone-only meeting by default', () => {
  for (const value of [null, undefined, '', 'unknown']) {
    const controller = create();
    load(controller, value);
    assert.equal(controller.snapshot().ready, false);
    assert.throws(() => args(controller), /Choose microphone/);
  }
});

test('system audio is requested without any call detection and without degraded capture', () => {
  const controller = create();
  load(controller, 'microphone-and-system');
  assert.deepEqual(args(controller), { intent: 'call', source: 'manual', allowDegraded: false });
});

test('an explicit microphone choice wins over a detected call', () => {
  const controller = create();
  load(controller, 'microphone');
  assert.deepEqual(args(controller, { detected: true }), { intent: 'room', source: 'manual' });
});

test('a detected call keeps its session association only when both sources are chosen', () => {
  const controller = create();
  load(controller, 'microphone-and-system');
  assert.deepEqual(args(controller, { detected: true }), { intent: 'call', source: 'call_detect', allowDegraded: false });
});

test('recording is blocked while a new source choice is being saved', async () => {
  const saving = deferred();
  const controller = create(() => saving.promise);
  load(controller, 'microphone');
  const result = controller.save('microphone-and-system');
  assert.equal(controller.snapshot().pending, true);
  assert.throws(() => args(controller), /Choose microphone/);
  saving.resolve();
  await result;
  assert.equal(args(controller).intent, 'call');
});

test('failed persistence cannot start the newly selected route or silently reuse the old route', async () => {
  const controller = create(async () => { throw new Error('write denied'); });
  load(controller, 'microphone');
  await controller.save('microphone-and-system');
  assert.equal(controller.snapshot().choice, 'microphone');
  assert.throws(() => args(controller), /could not be saved/);
  load(controller, 'microphone');
  assert.throws(() => args(controller), /could not be saved/);
  await controller.save('microphone');
  assert.throws(() => args(controller), /could not be saved/);
});

test('a slow startup preference response cannot undo an acknowledged source change', async () => {
  const controller = create();
  const initialRead = controller.beginLoad();
  await controller.save('microphone-and-system');
  controller.load('microphone', initialRead);
  assert.equal(args(controller).intent, 'call');
});

test('a preference read started during a save cannot replace the completed save', async () => {
  const saving = deferred();
  const controller = create(() => saving.promise);
  const result = controller.save('microphone-and-system');
  const racingRead = controller.beginLoad();
  saving.resolve();
  await result;
  controller.load('microphone', racingRead);
  assert.equal(args(controller).intent, 'call');
});

test('out-of-order device scans cannot restore an older source preference', () => {
  const controller = create();
  const older = controller.beginLoad(), newer = controller.beginLoad();
  controller.load('microphone-and-system', newer);
  controller.load('microphone', older);
  assert.equal(args(controller).intent, 'call');
});

test('the shipped Meeting button delegates to the explicit source controller', () => {
  const html = readFileSync(new URL('../tauri/src/index.html', import.meta.url), 'utf8');
  const begin = html.indexOf('    function recordingStartArgsForCurrentUi()');
  const end = html.indexOf('    let pendingRecordingStartArgs', begin);
  assert(begin > 0 && end > begin);
  const controller = create();
  load(controller, 'microphone-and-system');
  const context = vm.createContext({ recordingAudioChoice: controller, activeCallSession: null });
  vm.runInContext(html.slice(begin, end), context);
  const payload = vm.runInContext('recordingStartArgsForCurrentUi()', context);
  assert.deepEqual(JSON.parse(JSON.stringify(payload)), { intent: 'call', source: 'manual', allowDegraded: false });
});

test('the native-entry consent modal preserves the requested two-source route', () => {
  const html = readFileSync(new URL('../tauri/src/index.html', import.meta.url), 'utf8');
  const begin = html.indexOf("    window.__TAURI__.event.listen('minutes://recording-consent-required'");
  const end = html.indexOf('    function showRecordingStartError', begin);
  let listener, shown;
  vm.runInNewContext(html.slice(begin, end), {
    window: { __TAURI__: { event: { listen: (_name, callback) => { listener = callback; } } } },
    showRecordingConsentModal: (disclosure, args) => { shown = { disclosure, args }; },
  });
  const args = { mode: 'meeting', intent: 'call', allowDegraded: false, title: 'Synthetic call' };
  listener({ payload: { disclosure: 'Confirm consent', args } });
  assert.deepEqual(shown, { disclosure: 'Confirm consent', args });
});

test('onboarding shows the shared choice and cannot record until it and the model are ready', async () => {
  const html = readFileSync(new URL('../tauri/src/index.html', import.meta.url), 'utf8');
  const begin = html.indexOf('    function renderMeetingAudioChoice()');
  const end = html.indexOf("    meetingAudioSource.addEventListener", begin);
  const element = () => ({ value: '', disabled: false, textContent: '', dataset: {} });
  const mainPicker = element(), mainHelp = element(), onboardingPicker = element(), onboardingHelp = element();
  const button = element(); button.dataset.modelReady = 'true';
  const nodes = { 'onboarding-meeting-audio-source': onboardingPicker, 'onboarding-meeting-audio-help': onboardingHelp,
    'btn-start-first-recording': button };
  const saving = deferred(), controller = create(() => saving.promise);
  let active = false;
  const context = vm.createContext({ recordingAudioChoice: controller, meetingAudioSource: mainPicker, meetingAudioHelp: mainHelp,
    document: { getElementById: id => nodes[id] || null }, footer: { classList: { contains: () => active } }, syncPrimaryCaptureLabel: () => {} });
  vm.runInContext(`const renderedMeetingAudioChoices = new WeakMap(); ${html.slice(begin, end)}`, context);
  const render = () => vm.runInContext('renderMeetingAudioChoice()', context);
  render(); assert.equal(button.disabled, true);
  load(controller, 'microphone-and-system'); render();
  assert.equal(button.disabled, false);
  assert.equal(onboardingPicker.value, mainPicker.value);
  assert.match(onboardingHelp.textContent, /computer audio/);
  button.dataset.modelReady = 'false'; render(); assert.equal(button.disabled, true);
  button.dataset.modelReady = 'true';
  const result = controller.save('microphone'); render(); assert.equal(button.disabled, true);
  assert.equal(onboardingPicker.disabled, true);
  saving.resolve(); await result; render(); assert.equal(button.disabled, false);
  assert.match(onboardingHelp.textContent, /not included/);
  active = true; render(); assert.equal(button.disabled, true);
});

test('the first recording action uses the acknowledged audio choice', async () => {
  const html = readFileSync(new URL('../tauri/src/index.html', import.meta.url), 'utf8');
  const handler = html.match(/root\.querySelector\('#btn-start-first-recording'\)\.addEventListener\('click', async \(\) => \{[\s\S]*?\n      \}\);/);
  assert(handler);
  const controller = create(); load(controller, 'microphone-and-system');
  let listener, payload;
  vm.runInNewContext(handler[0], {
    root: { querySelector: () => ({ addEventListener: (_event, callback) => { listener = callback; } }) },
    setFooterCaptureMode: () => {}, startFastPoll: () => {},
    startDesktopRecordingFromUi: async () => { payload = args(controller); },
    showRecordingStartError: error => { throw error; }, console,
  });
  await listener();
  assert.equal(payload.intent, 'call');
  assert.equal(payload.allowDegraded, false);
});
