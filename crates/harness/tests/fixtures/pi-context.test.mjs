import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import register from '../../src/acp/pi_context.mjs';

test('Pi context bridge is opt-in and publishes native context including unknown after compaction', () => {
  const original = process.env.ZERON_PI_CONTEXT_DIR;
  const directory = mkdtempSync(join(tmpdir(), 'zeron-pi-context-test-'));
  try {
    delete process.env.ZERON_PI_CONTEXT_DIR;
    register({ on() { assert.fail('ordinary Pi must stay inert'); } });

    process.env.ZERON_PI_CONTEXT_DIR = directory;
    const hooks = new Map();
    register({ on(event, handler) { hooks.set(event, handler); } });
    let usage = { tokens: 32000, contextWindow: 200000 };
    const ctx = {
      sessionManager: { getSessionId: () => 'test-session' },
      getContextUsage: () => usage,
      model: { contextWindow: 200000 },
    };
    const read = () => JSON.parse(readFileSync(join(directory, 'test-session.json'), 'utf8'));
    hooks.get('session_start')({}, ctx);
    assert.deepEqual(read(), { sessionId: 'test-session', tokens: 32000, window: 200000 });
    usage = { tokens: null, contextWindow: 200000 };
    hooks.get('session_compact')({}, ctx);
    assert.equal(read().tokens, null);
    usage = { tokens: 8000, contextWindow: 128000 };
    hooks.get('agent_end')({}, ctx);
    assert.deepEqual(read(), { sessionId: 'test-session', tokens: 8000, window: 128000 });
    assert.ok(hooks.has('session_tree') && hooks.has('model_select'));
    ctx.sessionManager.getSessionId = () => '../foreign';
    hooks.get('message_end')({}, ctx);
    assert.deepEqual(readdirSync(directory), ['test-session.json']);
  } finally {
    if (original === undefined) delete process.env.ZERON_PI_CONTEXT_DIR;
    else process.env.ZERON_PI_CONTEXT_DIR = original;
    rmSync(directory, { recursive: true, force: true });
  }
});
