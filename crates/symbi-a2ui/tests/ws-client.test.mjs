import { readFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
import test from 'node:test';
import ts from 'typescript';

// Execute the production modules without introducing a second browser build pipeline.
const encode = code => `data:text/javascript;base64,${Buffer.from(code).toString('base64')}`;
async function compile(name) {
  const source = await readFile(new URL(`../src/api/${name}.ts`, import.meta.url), 'utf8');
  return ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  }).outputText;
}
const client = encode(await compile('client'));
const { WsClient } = await import(encode((await compile('ws-client')).replace('./client.js', client)));

function fixture(t) {
  const original = new Map(['window', 'localStorage', 'fetch', 'WebSocket'].map(k => [k, globalThis[k]]));
  const sockets = [];
  const requests = [];
  globalThis.window = { location: { protocol: 'https:', host: 'runtime.example' } };
  globalThis.localStorage = { getItem: () => 'admin.synthetic-credential' };
  globalThis.fetch = async (url, init) => {
    requests.push({ url, init });
    return new Response(JSON.stringify({ session: `ticket-${requests.length}`, expires_in: 60 }));
  };
  globalThis.WebSocket = class {
    static OPEN = 1;
    static CONNECTING = 0;
    readyState = 0;
    constructor(url) { this.url = url; sockets.push(this); }
    close() { this.readyState = 3; this.onclose?.(); }
  };
  const ws = new WsClient();
  t.after(() => {
    ws.disconnect();
    for (const [key, value] of original) {
      if (value === undefined) delete globalThis[key];
      else globalThis[key] = value;
    }
  });
  return { ws, sockets, requests };
}

const settle = () => new Promise(resolve => setImmediate(resolve));

test('bearer is sent only in the exchange header; reconnect obtains a new ticket', async t => {
  const { ws, sockets, requests } = fixture(t);
  ws.connect();
  await settle();
  assert.equal(requests[0].url, '/api/v1/auth/ws-session');
  assert.equal(requests[0].init.method, 'POST');
  assert.equal(requests[0].init.headers.Authorization, 'Bearer admin.synthetic-credential');
  assert.equal(sockets[0].url, 'wss://runtime.example/ws/chat?session=ticket-1');
  sockets[0].onopen();
  assert.equal(ws.state, 'connected');
  sockets[0].onclose();
  await new Promise(resolve => setTimeout(resolve, 1100));
  assert.equal(requests.length, 2);
  assert.equal(sockets[1].url, 'wss://runtime.example/ws/chat?session=ticket-2');
});

test('disconnect during an exchange cannot reopen the connection', async t => {
  const { ws, sockets } = fixture(t);
  let finish;
  globalThis.fetch = () => new Promise(resolve => { finish = resolve; });
  ws.connect();
  ws.disconnect();
  finish(new Response(JSON.stringify({ session: 'stale' })));
  await settle();
  assert.equal(sockets.length, 0);
  assert.equal(ws.state, 'disconnected');
});

test('events from a disconnected socket cannot change current connection state', async t => {
  const { ws, sockets } = fixture(t);
  ws.connect();
  await settle();
  const old = sockets[0];
  ws.disconnect();
  ws.connect();
  await settle();
  sockets[1].onopen();
  old.onerror();
  old.onclose();
  old.onopen();
  assert.equal(ws.state, 'connected');
});
