// The screen has two slots that share ~10 MB: startup = intro (plays once at power-on, start_video.mp4),
// standby = loop (plays forever after the intro, sleep_video.mp4). Mode "loop" = standby slot with a blank intro.
const settings = require('./settings');

const UNKNOWN_OTHER = 1.8e6; // assumed size of a slot we have never written
const BLANK_SIZE = 20000;    // size we reserve for the black clip
const isBlank = v => !!v && v.size <= 100000;

function budgets(s = settings.load()) {
  const total = s.totalBudgetMB * 1e6;
  const su = s.slots.startup, sb = s.slots.standby;
  return {
    total,
    loop: Math.floor(total - BLANK_SIZE), // intro is (or will be made) blank
    startup: Math.floor(total - (sb ? sb.size : UNKNOWN_OTHER)),
    standby: Math.floor(total - (su ? su.size : UNKNOWN_OTHER)),
    both: Math.floor(total / 2),
    assumed: { startup: !sb, standby: !su },
    assumedBytes: UNKNOWN_OTHER,
    introBlank: isBlank(su),
    slots: s.slots,
  };
}

const budgetFor = (slot, s) => budgets(s)[['standby', 'both', 'loop'].includes(slot) ? slot : 'startup'];

module.exports = { budgets, budgetFor, isBlank, UNKNOWN_OTHER, BLANK_SIZE };
