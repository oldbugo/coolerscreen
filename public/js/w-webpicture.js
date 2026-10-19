// 4.0 W2 "Web picture" item (STUB: filled in by the webpicture worker). Registered by morewidgets.js. Same shape as w-todo.js / w-progress.js.
// Follow docs/v4/ui-style-guide.md section 4: helpers in `api.h` (field, select2, range, numInput, swRow) and `h.colour`; changes save
// at once through api.set / api.edited (no Save button); labels short, sentence case.
export default {
  type: 'webpicture', key: 'webpicture', name: 'Web picture', icon: '▦', group: 'Info',
  props: () => ({ color: '#ffffff' }),
  size: p => [0.3, 0.3],
  title: w => 'Web picture',
  panel(w, host, api, h) {
    host.append(h.colour('color', 'Colour', '#ffffff'));
    return true;
  },
};
