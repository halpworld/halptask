// The on-call widget (Tier C). It runs in a sandboxed iframe with no network
// and no access to the Hub except the one port the host hands it, over which
// it may make the `tasks_list` call its manifest declares.
//
// Protocol (docs/hub/P5-EXTENSIONS.md 6.3): the host posts a
// `ui/initialize` message carrying a MessagePort. Everything after that is
// JSON-RPC 2.0 on that port: we send `tools/call`, the host answers with
// `result.structuredContent`.
'use strict';

const MAX_ROWS = 20;
let port = null;
let nextId = 1;
const pending = new Map();

function status(text) {
  document.getElementById('status').textContent = text;
}

// Text is always set with textContent (never markup): task titles come
// from other people and tools.
function row(item) {
  const li = document.createElement('li');
  const fields = item && typeof item.fields === 'object' && item.fields ? item.fields : {};
  const sev = document.createElement('span');
  sev.className = 'sev';
  sev.textContent = 'SEV ' + String(fields.sev === undefined ? '?' : fields.sev);
  const title = document.createElement('span');
  title.textContent = String(item && item.title !== undefined ? item.title : '');
  const svc = document.createElement('span');
  svc.className = 'svc';
  svc.textContent = fields.service === undefined ? '' : String(fields.service);
  li.append(sev, title, svc);
  return li;
}

function render(items) {
  const list = document.getElementById('list');
  list.replaceChildren(...items.slice(0, MAX_ROWS).map(row));
  const more = items.length - MAX_ROWS;
  status(items.length === 0 ? 'No open incidents.' : items.length + ' open' + (more > 0 ? ' (showing ' + MAX_ROWS + ')' : ''));
  if (port) port.postMessage({ jsonrpc: '2.0', method: 'ui/size-changed', params: { h: document.body.scrollHeight } });
}

function call(name, args) {
  return new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    port.postMessage({ jsonrpc: '2.0', id, method: 'tools/call', params: { name, arguments: args } });
  });
}

async function refresh() {
  try {
    const res = await call('tasks_list', { type_id: 'acme.incident', status: ['todo', 'in_progress'] });
    const items = res && res.structuredContent && Array.isArray(res.structuredContent.items) ? res.structuredContent.items : [];
    render(items);
  } catch (e) {
    status('Could not read the incidents.');
  }
}

function onPort(e) {
  const m = e.data;
  if (!m || m.jsonrpc !== '2.0') return;
  if (m.id !== undefined && pending.has(m.id)) {
    const p = pending.get(m.id);
    pending.delete(m.id);
    if (m.error) p.reject(new Error(String(m.error.message)));
    else p.resolve(m.result);
  } else if (m.method === 'ui/notifications/data-changed') {
    refresh();
  }
}

window.addEventListener('message', (e) => {
  const m = e.data;
  // Only the host's handshake (a retry gives a fresh port).
  if (!m || m.method !== 'ui/initialize' || !e.ports || e.ports.length !== 1) return;
  port = e.ports[0];
  port.onmessage = onPort;
  refresh();
});
