// The screen stores two clips that share ~10 MB: startup = intro (plays once at power-on, start_video.mp4) and
// standby = loop (plays forever after the intro, sleep_video.mp4). The app always keeps its own short boot splash
// in the intro (see splash.js) and the user's clip in the loop; the intro is not exposed in the UI.
const settings = require('./settings');
const splash = require('./splash');

const UNKNOWN_OTHER = 1.8e6; // assumed size of a slot we have never written
const isBlank = v => !!v && v.size <= 100000;

function budgets(s = settings.load()) {
  const total = s.totalBudgetMB * 1e6;
  const intro = splash.sizeEstimate();
  return {
    total,
    loop: Math.floor(total - intro), // space for the user's clip (the intro holds the boot splash)
    intro,
    slots: s.slots,
  };
}

module.exports = { budgets, isBlank, UNKNOWN_OTHER };
