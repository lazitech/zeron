// Zeron-owned Pi context bridge. Inert outside a Zeron-launched Pi process.
import { writeFileSync, renameSync } from 'node:fs';
import { join } from 'node:path';

export default function (pi) {
  const directory = process.env.ZERON_PI_CONTEXT_DIR;
  if (!directory) return;

  function publish(_event, ctx) {
    try {
      const sessionId = ctx.sessionManager.getSessionId();
      if (!/^[a-zA-Z0-9-]{1,128}$/.test(sessionId)) return;
      // Pi accounts for the active branch, context edits and compaction.
      // null tokens immediately after compaction must clear the old reading.
      const usage = ctx.getContextUsage();
      const number = value => Number.isSafeInteger(value) && value >= 0 ? value : null;
      const snapshot = {
        sessionId,
        tokens: number(usage?.tokens),
        window: number(usage?.contextWindow ?? ctx.model?.contextWindow),
      };
      const destination = join(directory, `${sessionId}.json`);
      const temporary = `${destination}.${process.pid}.tmp`;
      writeFileSync(temporary, JSON.stringify(snapshot), { mode: 0o600 });
      renameSync(temporary, destination);
    } catch {
      // Optional telemetry must never interfere with a prompt or extension.
    }
  }

  for (const event of ['session_start', 'message_end', 'agent_end', 'session_compact',
    'session_tree', 'model_select', 'session_shutdown']) {
    pi.on(event, publish);
  }
}
