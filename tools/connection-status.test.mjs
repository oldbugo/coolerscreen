// Run: node tools/connection-status.test.mjs (pure state tests; no USB or app).
import assert from 'node:assert/strict';
import { screenStatus, rememberedScreen, connectionMessage, liveMessage, shouldWatchPresence, presenceArrival, deviceJobActive } from '../public/js/connection-status.js';

const manual = { state: 'failedUsb', info: null, error: 'old failure' };
const live = (state, extra = {}) => ({ settings: { inApp: true }, status: { running: true, connected: state === 'connected', connection: { state, message: 'last failure', retryInSeconds: 5, attempts: 3 }, ...extra } });
const recovered = screenStatus(manual, live('connected'));
assert.equal(recovered.state, 'ready');
assert.equal(recovered.error, null, 'recovery clears old error');
assert.equal(manual.error, 'old failure', 'derive status without altering manual history');
assert.equal(rememberedScreen(manual, live('connected')).state, 'ready');
assert.equal(rememberedScreen(manual, live('connected'), { checking: true }), manual, 'do not overwrite manual check');
assert.equal(rememberedScreen(manual, live('connected'), { deviceBusy: true }), manual, 'device job remains authoritative');
for (const state of ['notFound', 'failedUsb', 'busy', 'noReply', 'error']) {
  const actual = screenStatus({ state: 'ready', info: {} }, live(state));
  assert.equal(actual.state, state);
  assert.equal(actual.info, null, 'failed Live must not display stale screen info');
  assert.match(connectionMessage(actual), /Retrying in 5 s/);
}
assert.doesNotMatch(connectionMessage(screenStatus(manual, live('failedUsb'))), /startup|started/);
assert.match(connectionMessage(screenStatus(manual, live('failedUsb'))), /Windows approval/);
assert.equal(screenStatus(manual, live('connected'), { checking: true }).state, 'checking');
assert.equal(screenStatus(manual, live('connected'), { deviceBusy: true }).state, 'updating');
assert.equal(screenStatus(manual, live('failedUsb'), { jobs: [{ kind: 'send', status: 'waiting' }] }).state, 'updating');
assert.equal(deviceJobActive([{ kind: 'version', status: 'running' }]), false, 'CPU work is not a device hold');
for (const hold of ['paused', 'deviceHold', 'systemHold', 'idle', 'layerOff']) {
  assert.equal(screenStatus(manual, live('failedUsb', { [hold]: true })), manual, `ignore old Live failure while ${hold}`);
}
const off = { settings: { inApp: false }, status: { running: false, connection: { state: 'off' } } };
assert.equal(screenStatus(rememberedScreen(manual, live('connected')), off).state, 'ready', 'turning Live off keeps recovery, not the old failure');
assert.equal(screenStatus(manual, off), manual);
assert.equal(screenStatus(manual, live('connected', { twinning: true })), manual, 'Null sink is not USB health');
assert.equal(screenStatus(manual, live('connected', { noDevice: true })), manual, 'fake Null sink is not USB health');
assert.equal(screenStatus(manual, { settings: { inApp: false }, status: { previewing: true, connected: true } }).state, 'ready', 'real preview proves current connection');
assert.equal(screenStatus({ state: 'setup' }, live('error')).state, 'setup', 'preserve setup action for serial driver');
assert.equal(screenStatus(manual, live('unknown')).state, 'failedUsb', 'unknown protocol state is not healthy');
assert.equal(screenStatus(manual, live('busy', { connection: { state: 'busy', retryInSeconds: Infinity } })).retryInSeconds, 0);
assert.match(connectionMessage(screenStatus(manual, live('error', { connection: { state: 'error', retryInSeconds: -5 } }))), /Retrying automatically/);
assert.equal(liveMessage(off, manual), '');
assert.match(liveMessage(live('connected', { measuredFps: 59.5, fps: 60 }), recovered), /59.5 of 60/);
assert.match(liveMessage(live('failedUsb', { deviceHold: true }), recovered), /Paused/);
assert.match(liveMessage(live('connected'), { source: 'check' }), /Checking/);
assert.match(liveMessage(live('failedUsb'), screenStatus(manual, live('failedUsb'))), /Windows cannot recognise/);
assert.match(liveMessage(live('connected', { twinning: true }), recovered), /nothing is sent/);
assert.equal(shouldWatchPresence(manual, off), true);
assert.equal(shouldWatchPresence({ state: 'notFound' }, off), true);
assert.equal(shouldWatchPresence({ state: 'ready' }, off), false, 'no periodic opens of healthy screen');
assert.equal(shouldWatchPresence(manual, live('failedUsb')), false, 'Live owns retrying');
assert.equal(shouldWatchPresence(manual, off, { checking: true }), false);
assert.equal(shouldWatchPresence(manual, off, { deviceBusy: true }), false);
assert.equal(shouldWatchPresence(manual, { ...off, status: { systemHold: true } }), false);
assert.equal(shouldWatchPresence(manual, { ...off, status: { twinning: true } }), true, 'editor-only preview does not hold USB');
const arrived = presenceArrival('none', { present: true, transport: 'winusb' });
assert.equal(arrived.arrived, true);
assert.equal(presenceArrival(arrived.key, { present: true, transport: 'winusb' }).arrived, false, 'do not repeatedly check a present but unresponsive screen');
const removed = presenceArrival(arrived.key, { present: false, transport: 'none' });
assert.equal(removed.arrived, false);
assert.equal(presenceArrival(removed.key, { present: true, transport: 'serial' }).arrived, true, 'return under serial driver can show setup');
console.log('Connection status state, recovery, hold, retry and presence tests passed.');
