// Minimal Chrome DevTools Protocol client for a quiet CoolerScreen test instance (WebView2 debug port).
// Used by shots.mjs; also a CLI:
//   node cdp.mjs <port> eval "<js>"            prints the JSON result (promises are awaited)
//   node cdp.mjs <port> shot <out.png> [w h]   screenshot at an emulated viewport (default 1280x860)
export async function connect(port) {
  const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
  const page = list.find(t => t.type === 'page');
  if (!page) throw new Error('no page on port ' + port);
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((res, rej) => { ws.addEventListener('open', res); ws.addEventListener('error', rej); });
  let id = 0;
  const pending = new Map();
  const errors = [];
  ws.addEventListener('message', ev => {
    const m = JSON.parse(ev.data);
    if (m.method === 'Runtime.exceptionThrown') errors.push(m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text);
    if (m.method === 'Runtime.consoleAPICalled' && m.params.type === 'error') errors.push(m.params.args.map(a => a.value ?? a.description).join(' '));
    if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
  });
  const send = (method, params = {}) => new Promise(r => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
  const c = {
    send,
    errors,
    async size(w, h) { await send('Emulation.setDeviceMetricsOverride', { width: +w, height: +h, deviceScaleFactor: 1, mobile: false }); },
    async eval(expression) {
      const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
      if (r.result && r.result.exceptionDetails) throw new Error('eval failed: ' + JSON.stringify(r.result.exceptionDetails).slice(0, 600));
      return r.result && r.result.result ? r.result.result.value : undefined;
    },
    async shot(file, w, h) {
      const { writeFileSync } = await import('node:fs');
      if (w) await c.size(w, h);
      await new Promise(r => setTimeout(r, 350));
      const r = await send('Page.captureScreenshot', { format: 'png' });
      writeFileSync(file, Buffer.from(r.result.data, 'base64'));
    },
    async key(key, code, vk, modifiers = 0) {
      await send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key, code, windowsVirtualKeyCode: vk, modifiers });
      await send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode: vk, modifiers });
    },
    close() { ws.close(); },
  };
  return c;
}

if (import.meta.url === `file:///${process.argv[1].replace(/\\/g, '/')}` || process.argv[1].endsWith('cdp.mjs')) {
  const [port, cmd, ...args] = process.argv.slice(2);
  if (port && cmd) {
    const c = await connect(port);
    if (cmd === 'eval') console.log(JSON.stringify(await c.eval(args[0]), null, 1));
    else if (cmd === 'shot') { await c.shot(args[0], args[1] || 1280, args[2] || 860); console.log('saved', args[0]); }
    c.close();
  }
}
