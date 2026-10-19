// 4.0 W1 "Sun and moon" item (STUB: filled in by the sunmoon worker). Registered by morewidgets.js. Follow docs/v4/ui-style-guide.md
// section 4: use the helpers in `api.h` (field, select2, range, numInput, swRow) and `h.colour`; changes save at once through
// api.set / api.edited (no Save button); labels short, sentence case.
export default {
  type: 'sunmoon', key: 'sunmoon', name: 'Sun and moon', icon: '▦', group: 'Info',
  props: () => ({ color: '#ffffff' }),
  /** Default box [w, h] (fractions of the 2400x1080 canvas). */
  size: p => [0.26, 0.2],
  title: w => 'Sun and moon',
  /** Append the Properties controls to `host`; return true. `h`: { colour(key, text, dflt, resettable), restructure(fn), clamp, rateField() }. */
  panel(w, host, api, h) {
    host.append(h.colour('color', 'Colour', '#ffffff'));
    return true;
  },
};
