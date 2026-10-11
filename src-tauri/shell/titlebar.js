// Seamless frameless window chrome (vault pattern "Seamless frameless window", Rhine Music):
// a transparent 32 px HTML title bar with three Windows 11-style buttons (46 x 32), injected into every page
// the shell shows, so public/ stays unchanged. The app's own top bar (its empty parts) is the drag region.
(function () {
  if (window.__csChrome) return;
  window.__csChrome = true;
  const T = () => window.__TAURI_INTERNALS__;
  const inv = (cmd, args) => T().invoke(cmd, args || {});
  const listen = (event, fn) =>
    inv('plugin:event|listen', { event, target: { kind: 'Any' }, handler: T().transformCallback(e => fn(e.payload)) });

  const ICON = {
    min: '<svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 5h10" stroke="currentColor" stroke-width="1"/></svg>',
    max: '<svg width="10" height="10" viewBox="0 0 10 10"><rect x=".5" y=".5" width="9" height="9" rx="1" fill="none" stroke="currentColor"/></svg>',
    restore: '<svg width="10" height="10" viewBox="0 0 10 10"><rect x=".5" y="2.5" width="7" height="7" rx="1" fill="none" stroke="currentColor"/><path d="M2.5 2.5V1.5a1 1 0 0 1 1-1h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1h-1" fill="none" stroke="currentColor"/></svg>',
    close: '<svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" stroke-width="1"/></svg>',
  };
  const CSS = `
#cs-titlebar{position:fixed;top:0;left:0;right:0;height:32px;z-index:2147483000;display:flex;justify-content:flex-end;
  background:transparent;pointer-events:none;-webkit-user-select:none;user-select:none}
#cs-titlebar button{pointer-events:auto;width:46px;height:32px;border:0;margin:0;padding:0;border-radius:0;display:grid;
  place-items:center;background:transparent;color:var(--cs-tb-fg,#e8eaee);cursor:default;outline:none}
#cs-titlebar button:hover{background:rgba(255,255,255,.08)}
#cs-titlebar button:active{background:rgba(255,255,255,.05)}
#cs-titlebar #cs-close:hover{background:#c42b1c;color:#fff}
#cs-titlebar #cs-close:active{background:#a82116;color:#fff}
@media (prefers-color-scheme: light){:root:not(.cs-dark-ui) #cs-titlebar button:hover{background:rgba(0,0,0,.06)}}
.topbar{padding-right:calc(16px + 138px) !important}`;

  function mount() {
    if (document.getElementById('cs-titlebar')) return;
    const st = document.createElement('style');
    st.textContent = CSS;
    document.head.appendChild(st);
    const h = document.createElement('div');
    h.id = 'cs-titlebar';
    h.innerHTML =
      `<button id="cs-min" type="button" aria-label="Minimise" tabindex="-1">${ICON.min}</button>` +
      `<button id="cs-max" type="button" aria-label="Maximise" tabindex="-1">${ICON.max}</button>` +
      `<button id="cs-close" type="button" aria-label="Close" tabindex="-1">${ICON.close}</button>`;
    document.body.appendChild(h);
    // Tauri drags only when the pressed element itself carries the attribute, so buttons inside stay clickable.
    const markDrag = () =>
      document.querySelectorAll('.topbar, .topbar .appname, .topbar .spacer, [data-cs-drag]')
        .forEach(e => e.setAttribute('data-tauri-drag-region', ''));
    markDrag();
    new MutationObserver(markDrag).observe(document.body, { childList: true });
    const maxBtn = h.querySelector('#cs-max');
    const setMax = m => {
      maxBtn.innerHTML = m ? ICON.restore : ICON.max;
      maxBtn.setAttribute('aria-label', m ? 'Restore' : 'Maximise');
    };
    h.querySelector('#cs-min').onclick = () => inv('win_minimize');
    maxBtn.onclick = () => inv('win_toggle_maximize').then(setMax);
    h.querySelector('#cs-close').onclick = () => inv('win_close');
    window.addEventListener('keydown', e => {
      if (e.key === 'F11') { e.preventDefault(); inv('win_toggle_maximize').then(setMax); }
    });
    listen('window-state', p => setMax(!!(p && p.maximized)));
    listen('close-guard', () => {
      if (confirm('An upload is in progress.\n\nIf you close now, the screen may show a broken picture until you send again.\n\nClose anyway?'))
        inv('win_force_close');
    });
    inv('win_state').then(setMax).catch(() => {});
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mount);
  else mount();
})();
