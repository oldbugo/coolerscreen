// The screen stores two clips that share ~10 MB: startup = intro (plays once at power-on, start_video.mp4) and
// standby = loop (plays forever after the intro, sleep_video.mp4). The app always keeps its own short boot splash
// in the intro (see splash.js) and the user's clip in the loop; the intro is not exposed in the UI.
const settings = require('./settings');
const splash = require('./splash');

const UNKNOWN_OTHER = 1.8e6; // assumed size of a slot we have never written
const SAFETY = 160000;       // margin kept free when the capacity was measured
const isBlank = v => !!v && v.size <= 100000;

function budgets(s = settings.load()) {
  // Measured capacity (free space + what our two clips occupy, at the last screen check), else the old fixed 9.5 MB.
  const measured = !!s.capacity;
  const total = measured ? s.capacity.bytes : s.totalBudgetMB * 1e6;
  const intro = splash.sizeEstimate();
  return {
    total, measured, capacityAt: measured ? s.capacity.at : null,
    loop: Math.floor(total - intro - (measured ? SAFETY : 0)), // space for the user's clip (the intro holds the boot splash)
    intro,
    slots: s.slots,
  };
}

/** From a screen info reply: capacity = free blocks + the sizes we recorded for both slots (null if unknown). */
function capacityFrom(info, s = settings.load()) {
  const free = Number(info && info.i_block_free), bs = Number(info && info.i_block_size) || 512;
  const su = s.slots.startup, sb = s.slots.standby;
  if (!Number.isFinite(free) || !su || !sb || sb.incomplete || su.incomplete) return null;
  return free * bs + su.size + sb.size;
}

module.exports = { budgets, capacityFrom, isBlank, UNKNOWN_OTHER, SAFETY };
