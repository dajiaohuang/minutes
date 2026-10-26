import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, openSync, closeSync, ftruncateSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import assert from 'node:assert/strict';

const profile = mkdtempSync(join(tmpdir(), 'minutes-mcp-model-preferences-'));
const models = join(profile, 'models');
mkdirSync(models);
const fd = openSync(join(models, 'ggml-base.bin'), 'w');
ftruncateSync(fd, 136 * 1024 * 1024);
closeSync(fd);
for (const name of ['ggml-silero-v6.2.0.bin', 'silero-vad-v6.2.0.onnx']) {
  writeFileSync(join(models, name), 'setup existing-file fixture, not an inference model');
}
const config = join(profile, 'config.toml');
const text = model => `# preserve this concurrent edit\n[transcription]\nengine = "whisper"\nmodel = "${model}"\nmodel_path = ${JSON.stringify(models)}\n[dictation]\nmodel = "small"\n[live_transcript]\nmodel = "base"\n`;
Object.assign(process.env, {
  MINUTES_CONFIG_PATH: config, MINUTES_HOME: join(profile, 'home'),
  MINUTES_DATA_DIR: join(profile, 'data'), XDG_CONFIG_HOME: join(profile, 'xdg'),
  MEETINGS_DIR: join(profile, 'meetings'), MINUTES_MCP_AUTO_SETUP: '0',
});
try {
  const { ensureWhisperModel } = await import('../dist/index.js');
  const logs = [];
  writeFileSync(config, text('base'));
  await ensureWhisperModel({
    autoSetup: '1', checkState: { done: false }, log: x => logs.push(x),
    health: async () => {
      // The selected model changes after the readiness observation. Actual
      // install-only CLI execution must not overwrite that later user edit.
      writeFileSync(config, text('medium'));
      return JSON.stringify({ ok: true, data: {
        engine: 'whisper', effective_engine: 'whisper', model: 'base',
        items: [{ label: 'Speech model', state: 'attention', detail: 'fixture' }],
      } });
    },
  });
  assert.equal(readFileSync(config, 'utf8'), text('medium'));
  assert(logs.some(x => x.includes('Whisper base model downloaded')), logs.join('\n'));
  assert(!logs.some(x => x.includes('Model download failed')), logs.join('\n'));
  console.log('PASS: actual MCP background setup preserves concurrent batch/live/dictation preferences byte-for-byte');
} finally {
  rmSync(profile, { recursive: true, force: true });
}
