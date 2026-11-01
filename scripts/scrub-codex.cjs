// Explicit projection of private recorder logs into public replay fixtures.
// Unknown protocol shapes stop the scrub so a reviewer can extend the projection.
const fs = require('node:fs');
const [scenario, input, output] = process.argv.slice(2);
const answers = {
  'full-turn': ['FIRST'], 'two-turns': ['FIRST', 'SECOND'],
  'approval-allow': ['FIXTURE_DONE'], 'approval-reject': ['', 'REPLACEMENT'],
  resume: ['ACK', 'ORBIT'], 'cancel-tool': [''],
};
if (!answers[scenario] || !input || !output || input === output) {
  throw Error('usage: node scripts/scrub-codex.cjs <scenario> <private.jsonl> <public.jsonl>');
}
const processes = new Map(), sessions = new Map(), tools = new Map(), turns = new Map(), texts = new Map();
const rows = [];
const alias = (map, value, prefix) => {
  if (typeof value !== 'string') throw Error(`missing ${prefix} id`);
  if (!map.has(value)) map.set(value, `${prefix}-${map.size + 1}`);
  return map.get(value);
};
const pick = (value, keys) => Object.fromEntries(keys.filter(k => value[k] !== undefined).map(k => [k, value[k]]));
const capabilityKeys = new Set(['auth', 'logout', 'providers', 'loadSession', 'promptCapabilities',
  'embeddedContext', 'image', 'audio', 'sessionCapabilities', 'resume', 'list', 'close', 'delete',
  'fork', 'additionalDirectories', 'subagents', 'systemPrompt', 'mcpCapabilities', 'acp', 'http', 'sse']);
function capabilities(value) {
  return Object.fromEntries(Object.entries(value).filter(([k]) => capabilityKeys.has(k)).map(([k, v]) =>
    [k, typeof v === 'boolean' ? v : capabilities(v)]));
}
function tool(value) {
  const result = pick(value, ['sessionUpdate', 'kind', 'status', 'name']);
  if (value.toolCallId !== undefined) result.toolCallId = alias(tools, value.toolCallId, 'tool');
  if (value.title !== undefined) result.title = 'fixture tool';
  return result;
}
function message(m, process, direction) {
  if (m.error) throw Error('RPC error needs manual review');
  const result = {jsonrpc: '2.0', ...pick(m, ['id', 'method'])};
  if (m.id !== undefined && !(typeof m.id === 'number' || /^\d+$/.test(m.id))) {
    throw Error('non-numeric RPC id needs manual review');
  }
  const p = m.params;
  if (m.method === 'initialize') {
    result.params = {protocolVersion: p.protocolVersion,
      clientCapabilities: {fs: {readTextFile: false, writeTextFile: false}, terminal: false},
      clientInfo: {name: 'astrolabe', version: '0.0.0'}};
  } else if (m.method === 'session/new') {
    result.params = {cwd: '/fixture/project', mcpServers: []};
  } else if (['session/resume', 'session/load'].includes(m.method)) {
    result.params = {cwd: '/fixture/project', sessionId: alias(sessions, p.sessionId, 'session'), mcpServers: []};
  } else if (m.method === 'session/prompt') {
    const turn = (turns.get(process) || 0) + 1;
    turns.set(process, turn);
    result.params = {prompt: [{type: 'text', text: `fixture prompt ${turn}`}], sessionId: alias(sessions, p.sessionId, 'session')};
  } else if (m.method === 'session/cancel') {
    result.params = {sessionId: alias(sessions, p.sessionId, 'session')};
  } else if (m.method === '_auth/status_update') {
    result.params = {};
  } else if (m.method === 'session/request_permission') {
    result.params = {sessionId: alias(sessions, p.sessionId, 'session'), toolCall: tool(p.toolCall),
      options: p.options.map(o => {
        if (!['allow_once', 'allow_always', 'reject_once', 'reject_always'].includes(o.kind)) throw Error('unknown option kind');
        if (!['allow_once', 'accept_execpolicy_amendment', 'cancel'].includes(o.optionId)) throw Error('unknown option id');
        return {optionId: o.optionId, kind: o.kind, name: o.kind};
      })};
  } else if (m.method === 'session/update') {
    const u = p.update, kind = u.sessionUpdate;
    let update = {sessionUpdate: kind};
    if (['tool_call', 'tool_call_update'].includes(kind)) update = tool(u);
    else if (kind === 'agent_message_chunk') {
      if (u.content.type !== 'text') throw Error('non-text answer');
      const key = `${process}:${turns.get(process)}`;
      const text = (texts.get(key) || '') + u.content.text;
      const expected = answers[scenario][scenario === 'resume' ? process : turns.get(process) - 1];
      if (expected === undefined || !expected.startsWith(text)) throw Error('unexpected answer text');
      texts.set(key, text);
      update.content = {type: 'text', text: u.content.text};
    } else if (kind === 'available_commands_update') update.availableCommands = [];
    else if (kind === 'usage_update') Object.assign(update, {size: 1000, used: 100});
    else if (kind !== 'session_info_update') throw Error('unknown session update');
    result.params = {sessionId: alias(sessions, p.sessionId, 'session'), update};
  } else if (m.method) throw Error('unknown method');
  else if (m.result?.protocolVersion !== undefined) {
    result.result = {protocolVersion: m.result.protocolVersion, agentCapabilities: capabilities(m.result.agentCapabilities)};
    result.result.agentCapabilities._meta = {authStatus: {}};
  } else if (m.result?.sessionId !== undefined) {
    result.result = {sessionId: alias(sessions, m.result.sessionId, 'session')};
  } else if (m.result?.stopReason !== undefined) result.result = {stopReason: m.result.stopReason};
  else if (m.result?.outcome) {
    const o = m.result.outcome;
    if (!['allow_once', 'accept_execpolicy_amendment', 'cancel'].includes(o.optionId)) throw Error('unknown selected option');
    result.result = {outcome: pick(o, ['optionId', 'outcome'])};
  } else if (m.result && Object.keys(m.result).length === 0) result.result = {};
  else throw Error(`unknown ${direction} response`);
  return result;
}
const stderrSeen = new Set();
for (const line of fs.readFileSync(input, 'utf8').trim().split('\n')) {
  const r = JSON.parse(line);
  if (!['wire', 'exit'].includes(r.kind)) continue;
  if (!processes.has(r.process)) processes.set(r.process, processes.size);
  const process = processes.get(r.process);
  if (r.kind === 'exit') {
    if (!Number.isInteger(r.code)) throw Error('exit without numeric code');
    rows.push({process, exit_code: r.code});
  } else if (r.direction === 'stderr') {
    if (!stderrSeen.has(process) && r.line.includes('AppServerUnavailableError: The Codex app-server exited with code 0. The agent is shutting down.')) {
      stderrSeen.add(process);
      rows.push({process, stderr: 'AppServerUnavailableError: The Codex app-server exited with code 0. The agent is shutting down.\n'});
    }
  } else if (['to_agent', 'from_agent'].includes(r.direction)) {
    rows.push({process, elapsed_ms: r.elapsed_ms, direction: r.direction,
      message: message(JSON.parse(r.line), process, r.direction)});
  } else throw Error('unknown wire direction');
}
// Write only after all rows have passed the projection. Never overwrite a file.
fs.writeFileSync(output, rows.map(r => JSON.stringify(r)).join('\n') + '\n', {flag: 'wx'});
