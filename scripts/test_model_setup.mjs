import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import test from 'node:test';

const html = readFileSync(new URL('../tauri/src/index.html', import.meta.url), 'utf8');
const start = html.indexOf('    async function installOnboardingModels(');
const end = html.indexOf('    function setupOnboarding(', start);
assert(start > 0 && end > start);
const source = html.slice(start, end);

async function install(settings, failModel) {
  const calls = [];
  let readinessReads = 0;
  const result = { hasModel: true, needsSetup: false, activation: { hasModel: true } };
  const context = vm.createContext({ invoke: async (name, args) => {
    calls.push({ name, args });
    if (name === 'cmd_download_model' && args.model === failModel) throw new Error('fixture download failed');
    if (name === 'cmd_needs_setup') {
      if (readinessReads++ === 0) return settings === null ? {} : { modelSelections: {
        dictation: settings.dictation?.model ?? 'base',
        standaloneLive: settings.live_transcript?.model ?? '',
      } };
      return result;
    }
    return 'ok';
  } });
  vm.runInContext(source, context);
  try {
    const setup = await vm.runInContext('installOnboardingModels("small")', context);
    return { calls: JSON.parse(JSON.stringify(calls)), setup, result };
  } catch (error) { return { calls: JSON.parse(JSON.stringify(calls)), error }; }
}

test('meeting onboarding installs Small and Base while selecting only the batch model', async () => {
  const { calls, setup, result } = await install({ dictation: { model: 'base' }, live_transcript: { model: '' } });
  assert.deepEqual(calls.filter(x => x.name === 'cmd_download_model').map(x => x.args.model), ['small', 'base']);
  assert.deepEqual(calls.filter(x => x.name === 'cmd_set_setting'), [
    { name: 'cmd_set_setting', args: { section: 'transcription', key: 'model', value: 'small' } },
  ]);
  assert.equal(setup, result);
  assert.equal(calls.some(x => x.name === 'cmd_get_settings'), false);
});

test('explicit live and dictation choices survive onboarding, including low-resource Tiny', async () => {
  const { calls } = await install({ dictation: { model: 'tiny' }, live_transcript: { model: 'medium' } });
  assert.deepEqual(calls.filter(x => x.name === 'cmd_download_model').map(x => x.args.model), ['small', 'tiny', 'medium']);
  assert.equal(calls.filter(x => x.name === 'cmd_set_setting').length, 1);
  assert.equal(calls.find(x => x.name === 'cmd_set_setting').args.section, 'transcription');
});

test('a failed companion install leaves every saved model selection unchanged', async () => {
  const { calls, error } = await install({ dictation: { model: 'base' } }, 'base');
  assert.match(error.message, /fixture download failed/);
  assert.equal(calls.some(x => x.name === 'cmd_set_setting'), false);
  assert.equal(calls.filter(x => x.name === 'cmd_needs_setup').length, 1);
});

test('custom model paths are preserved and never sent to the named-model downloader', async () => {
  const { calls } = await install({ dictation: { model: '/custom/voice.bin' }, live_transcript: { model: '/custom/live.bin' } });
  assert.deepEqual(calls.filter(x => x.name === 'cmd_download_model').map(x => x.args.model), ['small']);
});

test('missing preference metadata fails before downloads or selection changes', async () => {
  const { calls, error } = await install(null);
  assert.match(error.message, /Could not read your saved speech model choices/);
  assert.equal(calls.some(x => x.name === 'cmd_download_model' || x.name === 'cmd_set_setting'), false);
});

test('Tiny remains an explicit advanced setting, with no primary onboarding button', () => {
  assert(!html.includes('data-model="tiny"'));
  assert.match(html, /optgroup label="Advanced: lower memory, lower accuracy">\s*<option value="tiny">/);
  assert.match(html, /id="settings-tiny-model-note"/);
  assert.match(html, /data-model="small">Small \(466 MB, recommended\)/);
});

test('a historical download milestone cannot hide a currently missing model', () => {
  const nodes = new Map();
  const root = { querySelector: id => {
    if (!nodes.has(id)) {
      const classes = new Set();
      nodes.set(id, { textContent: '', dataset: {}, classes, classList: {
        add: x => classes.add(x), remove: x => classes.delete(x),
        toggle: (x, on) => on ? classes.add(x) : classes.delete(x),
      } });
    }
    return nodes.get(id);
  } };
  let audioRenders = 0;
  const context = vm.createContext({ root, setText: (node, text) => { node.textContent = text; },
    renderMeetingAudioChoice: () => { audioRenders++; } });
  const activationStart = html.indexOf('    function applyActivationState(');
  const activationEnd = html.indexOf('    function renderStandaloneLiveNote(', activationStart);
  assert(activationStart > 0 && activationEnd > activationStart);
  vm.runInContext(html.slice(activationStart, activationEnd), context);
  vm.runInContext('applyActivationState(root, { hasModel: true, milestones: { modelReadyAt: "past" } })', context);
  assert(root.querySelector('#step-model').classes.has('done'));
  assert.equal(root.querySelector('#btn-start-first-recording').dataset.modelReady, 'true');
  vm.runInContext('applyActivationState(root, { hasModel: false, milestones: { modelReadyAt: "past" } })', context);
  assert(!root.querySelector('#step-model').classes.has('done'));
  assert(!root.querySelector('#step-model-body').classes.has('ui-hidden'));
  assert(root.querySelector('#step-first-artifact').classes.has('disabled'));
  assert.equal(root.querySelector('#step-model-status').textContent, 'Needs setup');
  assert.equal(root.querySelector('#btn-start-first-recording').dataset.modelReady, 'false');
  assert.equal(audioRenders, 2);
});
