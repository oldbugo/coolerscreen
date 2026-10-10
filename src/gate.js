// One work queue for everything heavy: ffmpeg conversions/estimates/gallery scans ("convert") and anything that talks
// to the screen ("device"). Tasks run strictly one at a time in arrival order, so a conversion can never run while
// a screen upload is in progress (it stalled one once) and two encodes never compete.
const queue = [];
let running = null;

function messageFor(task) {
  const ahead = [running, ...queue.slice(0, queue.indexOf(task))].filter(Boolean);
  if (ahead.some(t => t.kind === 'device')) return 'Waiting for the screen operation to finish…';
  if (task.kind === 'device') return 'Waiting for conversions to finish…';
  return 'Waiting for the previous conversion to finish…';
}

function notifyWaiting() {
  for (const t of queue) if (t.onWait) t.onWait(messageFor(t));
}

function pump() {
  if (running || !queue.length) return;
  running = queue.shift();
  const t = running;
  if (t.onWait) t.onWait(null);
  Promise.resolve().then(t.fn).then(
    r => finish(t, r, null),
    e => finish(t, null, e),
  );
  notifyWaiting();
}

function finish(t, r, e) {
  running = null;
  if (e) t.reject(e); else t.resolve(r);
  pump();
  notifyWaiting();
}

/**
 * Run fn() when it is this task's turn. kind: 'convert' | 'device'.
 * opts.onWait(message|null): called with a waiting message while queued, and null when it starts.
 */
function run(kind, fn, opts = {}) {
  return new Promise((resolve, reject) => {
    const t = { kind, fn, resolve, reject, onWait: opts.onWait };
    queue.push(t);
    if (running) notifyWaiting();
    pump();
  });
}

/** True if a screen operation is running or queued. */
function deviceActive() {
  return (running && running.kind === 'device') || queue.some(t => t.kind === 'device');
}

module.exports = { run, deviceActive };
