// Geometry shared by every screen-shaped preview. Mirrors src/convert.js exactly (frameGraph, bandPx) so the instant
// preview shows the same crop, position and notch as the converted version. Units: the 2400x1080 landscape canvas.
export const W = 2400, H = 1080;

const trunc2 = v => 2 * Math.trunc(v / 2);

/** Zoom at which the whole picture is visible (black bars), relative to "cover" (= 1). */
export function fitZoom(w, h) {
  if (!w || !h) return 1;
  return Math.min(W / w, H / h) / Math.max(W / w, H / h);
}

/** Where the source picture lands on the canvas: { left, top, width, height } in canvas pixels. */
export function frameGeometry(w, h, f) {
  const z = f && Number.isFinite(f.zoom) ? f.zoom : 1;
  const fx = f && Number.isFinite(f.x) ? f.x : 0.5, fy = f && Number.isFinite(f.y) ? f.y : 0.5;
  const S = z * Math.max(W / w, H / h);
  const dw = trunc2(w * S), dh = trunc2(h * S);
  // ffmpeg: pad (picture smaller than the screen) then crop (picture larger); both offsets are rounded down to even.
  const left = dw < W ? trunc2((W - dw) * fx) : -trunc2((dw - W) * fx);
  const top = dh < H ? trunc2((H - dh) * fy) : -trunc2((dh - H) * fy);
  return { left, top, width: dw, height: dh };
}

/** Same rounding as src/convert.js bandPx(): the band edge lands on the 16 px MJPEG block grid. */
export function bandPx(band) {
  const px = Math.round(band.px);
  if (px <= 0) return 0;
  const offset = band.edge === 'top' ? (1080 % 16) : 0;
  return Math.ceil((px - offset) / 16) * 16 + offset;
}

/** Style the notch overlay element (absolute, inside a 20:9 frame) for a mask; hides it when the mask is off. */
export function styleNotch(el, mask) {
  const pct = (v, of) => (v / of) * 100 + '%';
  el.style.cssText = '';
  el.hidden = true;
  if (!mask || mask.mode === 'off') return;
  if (mask.mode === 'band') {
    const px = bandPx(mask.band);
    if (!px) return;
    const e = mask.band.edge;
    Object.assign(el.style, {
      left: e === 'right' ? pct(W - px, W) : '0', top: e === 'bottom' ? pct(H - px, H) : '0',
      width: e === 'left' || e === 'right' ? pct(px, W) : '100%', height: e === 'top' || e === 'bottom' ? pct(px, H) : '100%',
    });
    el.className = 'notch band';
  } else {
    const { cx, cy, r, feather } = mask.circle;
    if (r <= 0) return;
    const R = r + feather;
    Object.assign(el.style, { left: pct(cx - R, W), top: pct(cy - R, H), width: pct(2 * R, W), height: pct(2 * R, H) });
    el.style.background = feather > 0
      ? `radial-gradient(circle closest-side, #000 ${(r / R) * 100}%, rgba(0,0,0,0) 100%)` : '#000';
    el.className = 'notch circle';
  }
  el.hidden = false;
}
