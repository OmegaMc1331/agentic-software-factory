import { randomBytes } from "node:crypto";
import { chromium } from 'playwright';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const base = process.env.P01_BASE_URL ?? 'http://127.0.0.1:4387';
const auth = await fetch(base + '/api/auth/session', { method: 'POST', headers: { Origin: base, 'Content-Type': 'application/json' }, body: '{}' });
const session = await auth.json();
const cookie = auth.headers.getSetCookie().map(value => value.split(';')[0]).join('; ');
const headers = { Cookie: cookie, Origin: base, 'X-Factory-CSRF': session.csrfToken, 'Content-Type': 'application/json' };
const evidence = [];
const request = (suffix, method = 'GET', body, extra = {}) => fetch(base + '/api' + suffix, { method, headers: { ...headers, ...extra }, body: body === undefined ? undefined : JSON.stringify(body) });
async function rejected(name, promise, expected) { const response = await promise; assert.equal(response.status, expected, name); evidence.push({ name, status: response.status }); }

await rejected('CSRF required for mutation', request('/projects/small/state/codemirror', 'PUT', {}, { 'X-Factory-CSRF': '' }), 403);
await rejected('cross origin blocked', request('/projects', 'GET', undefined, { Origin: 'https://p01.invalid' }), 403);
await rejected('case variant private path blocked', request('/projects/other/file?path=.ENV'), 403);
await rejected('case variant Git metadata blocked', request('/projects/other/file?path=.GiT%2Fconfig'), 403);
const hostStatus = await new Promise((resolve, reject) => {
  const operation = http.request(base + '/api/projects', { headers: { ...headers, Host: 'p01.invalid:4387' } }, response => { response.resume(); resolve(response.statusCode); });
  operation.on('error', reject); operation.end();
});
assert.equal(hostStatus, 400); evidence.push({ name: 'DNS rebinding Host blocked using raw HTTP Host', status: hostStatus });
await rejected('unknown process profile blocked', request('/projects/other/protocol/arbitrary-shell', 'POST', {}), 400);
await rejected('oversized state payload bounded', request('/projects/other/state/codemirror', 'PUT', { content: 'x'.repeat(3 * 1024 * 1024) }), 413);

const lease = await (await request('/projects/other/protocol/python', 'POST', {})).json();
assert(lease.id);
await rejected('cross project process lease blocked', request(`/projects/small/trace/${lease.id}`), 403);
await rejected('duplicate process ownership blocked', request('/projects/other/protocol/python', 'POST', {}), 409);
async function upgrade(headersOverride) {
  return new Promise((resolve, reject) => {
    const request = http.request(`${base}/api/projects/other/transport/${lease.id}`, { headers: { ...headers, Connection: 'Upgrade', Upgrade: 'websocket', 'Sec-WebSocket-Version': '13', 'Sec-WebSocket-Key': randomBytes(16).toString('base64'), ...headersOverride } });
    request.on('response', response => { response.resume(); resolve(response.statusCode); });
    request.on('upgrade', (_response, socket) => { socket.destroy(); resolve(101); });
    request.on('error', reject); request.end();
  });
}
assert.equal(await upgrade({ Cookie: '' }), 401); evidence.push({ name: 'unauthenticated WebSocket blocked', status: 401 });
assert.equal(await upgrade({ Origin: 'https://p01.invalid' }), 403); evidence.push({ name: 'WebSocket origin blocked', status: 403 });

const browser = await chromium.launch({ channel: 'msedge', headless: true });
try {
  const page = await browser.newPage();
  await page.goto(base + '/codemirror/?project=other');
  await page.waitForFunction(() => window.p01?.workbench.ready);
  const projects = await (await request('/projects')).json();
  const project = projects.find(project => project.id === 'other');
  const messages = await page.evaluate(async ({ lease, project, foreign }) => {
    const socket = new WebSocket(`${location.origin.replace('http', 'ws')}/api/projects/other/transport/${lease.id}`);
    const values = [];
    socket.onmessage = event => values.push(JSON.parse(event.data));
    await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
    socket.send(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { processId: null, rootUri: project.uri, capabilities: {} } }));
    const wait = async predicate => { for (let index = 0; index < 300; index++) { if (predicate()) return; await new Promise(resolve => setTimeout(resolve, 50)); } throw new Error('real Pyright timeout'); };
    await wait(() => values.some(value => value.id === 1 && value.result));
    socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'initialized', params: {} }));
    socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'textDocument/didOpen', params: { textDocument: { uri: project.uri + 'debug.py', languageId: 'python', version: 1, text: 'value: int = "bad"\n' } } }));
    await wait(() => values.some(value => value.method === 'textDocument/publishDiagnostics' && value.params.diagnostics.length));
    socket.send(JSON.stringify({ jsonrpc: '2.0', id: 2, method: 'textDocument/hover', params: { textDocument: { uri: project.uri + 'debug.py' }, position: { line: 0, character: 2 } } }));
    await wait(() => values.some(value => value.id === 2 && value.result));
    socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'textDocument/didOpen', params: { textDocument: { uri: foreign + 'src/main.ts', languageId: 'typescript', version: 1, text: 'MUST_NOT_REACH_OTHER_PROJECT_SERVER' } } }));
    socket.send(JSON.stringify({ jsonrpc: '2.0', id: 3, method: 'workspace/executeCommand', params: { command: 'UNAUTHORIZED' } }));
    await wait(() => values.filter(value => value.type === 'policy-error').length >= 2);
    socket.close(); return values;
  }, { lease, project, foreign: projects.find(project => project.id === 'small').uri });
  const trace = await (await request(`/projects/other/trace/${lease.id}`)).json();
  assert.equal(trace.trace.filter(value => value.direction === 'client' && value.method === 'textDocument/didOpen').length, 1);
  assert(!trace.trace.some(value => value.direction === 'client' && value.method === 'workspace/executeCommand'));
  evidence.push({ name: 'real Pyright initialize diagnostics hover; foreign URI and executeCommand rejected before child stdin', messages, trace });
} finally {
  await request(`/projects/other/transport/${lease.id}`, 'DELETE');
  await browser.close();
}
fs.writeFileSync(path.join(root, '.factory/v2/p01/security-results.json'), JSON.stringify({ schemaVersion: 1, measuredAt: new Date().toISOString(), evidence }, null, 2));
console.log(`${evidence.length} security/protocol checks passed`);
