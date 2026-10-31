const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync, existsSync } = require('node:fs');
const { resolve } = require('node:path');

const root = resolve(__dirname, '../seed');
const read = (file) => readFileSync(resolve(root, file), 'utf8');
test('seed files, placeholders, ask rule, and disabled memories', () => {
  for (const file of ['AGENTS.md', 'owner.md', 'voice.md', '.mcp.json', '.codex/config.toml']) {
    assert.match(read(file), /PLACEHOLDER/);
  }
  for (const dir of ['efforts', 'journal', 'initiatives']) {
    assert.ok(existsSync(resolve(root, 'notes', dir, '.gitkeep')));
  }
  const agents = read('AGENTS.md');
  for (const phrase of ['owner.md', 'voice.md', 'one question per message', 'self-contained with context', 'recommendation and its reason', 'what waits', 'next only after an answer']) {
    assert.ok(agents.includes(phrase), phrase);
  }
  const settings = JSON.parse(read('.claude/settings.json'));
  assert.equal(settings.autoMemoryEnabled, false);
  assert.deepEqual(settings.hooks, {});
  assert.deepEqual(settings.permissions, { allow: [], ask: [], deny: [] });
  assert.match(read('.codex/config.toml'), /generate_memories = false/);
  assert.match(read('.codex/config.toml'), /use_memories = false/);
  assert.match(read('.codex/config.toml'), /default_tools_approval_mode = "approve"/);
  assert.match(read('.codex/config.toml'), /enabled = false/);
});
