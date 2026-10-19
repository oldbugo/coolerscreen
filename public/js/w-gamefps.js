// 4.0 G2 "Game FPS" item (STUB: filled in by the widget worker). Registered by morewidgets.js. Same shape as w-todo.js / w-headlines.js.
// Follow docs/v4/ui-style-guide.md section 4: helpers in `api.h` (field, select2, range, numInput, swRow) and `h.colour`; changes save at
// once through api.set / api.edited (no Save button); labels short, sentence case.
export default {
  type: 'gamefps', key: 'gamefps', name: 'Game FPS', icon: '▦', group: 'Sensors',
  props: () => ({ low: true, graph: true, history: 30, hideIdle: false, color: '#5ac8ff' }),
  size: p => [0.3, p.graph === false ? 0.2 : 0.36],
  title: w => 'Game FPS',
  panel(w, host, api, h) {
    host.append(h.colour('color', 'Colour', '#5ac8ff'));
    return true;
  },
};
