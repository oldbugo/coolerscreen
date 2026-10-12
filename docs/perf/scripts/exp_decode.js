// Loop-search frame extraction: CPU vs NVDEC/D3D11VA decode, fast-decode flags, one-pass multi-resolution.
// usage: node exp_decode.js <clip>
const path = require('path');
const { spawn } = require('child_process');
const { REPO, stage, startGpu, stopGpu } = require('./mon');
const ff = require(path.join(REPO, 'node_modules', 'ffmpeg-static'));

function run(args) {
  return new Promise((resolve, reject) => {
    const p = spawn(ff, ['-hide_banner', '-loglevel', 'error', '-nostdin', ...args], { windowsHide: true });
    let bytes = 0, err = '';
    p.stdout.on('data', d => (bytes += d.length));
    p.stderr.on('data', d => (err += d));
    p.on('close', c => (c === 0 ? resolve(bytes) : reject(new Error(err.slice(-300)))));
  });
}
const out = ['-an', '-f', 'rawvideo', 'pipe:1'];
(async () => {
  const f = path.resolve(process.argv[2]);
  const MAXF = ['-frames:v', '9000'];
  startGpu(); await new Promise(r => setTimeout(r, 800));
  const res = [];
  const v = {
    'cpu (1.2.0: scale area 64x36 gray)': ['-i', f, ...MAXF, '-fps_mode', 'passthrough', '-vf', 'scale=64:36:flags=area,format=gray', ...out],
    'cpu + skip_loop_filter + fast': ['-skip_loop_filter', 'all', '-flags2', 'fast', '-i', f, ...MAXF, '-fps_mode', 'passthrough', '-vf', 'scale=64:36:flags=area,format=gray', ...out],
    'cpu, scale fast_bilinear': ['-i', f, ...MAXF, '-fps_mode', 'passthrough', '-vf', 'scale=64:36:flags=fast_bilinear,format=gray', ...out],
    'nvdec cuda + scale_cuda 256x144 + area 64x36': ['-hwaccel', 'cuda', '-hwaccel_output_format', 'cuda', '-i', f, ...MAXF, '-fps_mode', 'passthrough', '-vf', 'scale_cuda=256:144,hwdownload,format=nv12,scale=64:36:flags=area,format=gray', ...out],
    'nvdec cuda (download full frames) + area': ['-hwaccel', 'cuda', '-i', f, ...MAXF, '-fps_mode', 'passthrough', '-vf', 'scale=64:36:flags=area,format=gray', ...out],
    'd3d11va (download) + area': ['-hwaccel', 'd3d11va', '-i', f, ...MAXF, '-fps_mode', 'passthrough', '-vf', 'scale=64:36:flags=area,format=gray', ...out],
    'nvdec + scale_cuda, 64x36 AND 192x108 in one pass': ['-hwaccel', 'cuda', '-hwaccel_output_format', 'cuda', '-i', f, ...MAXF, '-fps_mode', 'passthrough', '-filter_complex',
      '[0:v]scale_cuda=384:216,hwdownload,format=nv12,split[a][b];[a]scale=64:36:flags=area,format=gray[o1];[b]scale=192:108:flags=area,format=gray[o2]', '-map', '[o1]', '-f', 'rawvideo', 'pipe:1', '-map', '[o2]', '-f', 'rawvideo', 'NUL'],
    'cpu, 64x36 AND 192x108 in one pass': ['-i', f, ...MAXF, '-fps_mode', 'passthrough', '-filter_complex',
      '[0:v]split[a][b];[a]scale=64:36:flags=area,format=gray[o1];[b]scale=192:108:flags=area,format=gray[o2]', '-map', '[o1]', '-f', 'rawvideo', 'pipe:1', '-map', '[o2]', '-f', 'rawvideo', 'NUL'],
  };
  for (const [k, a] of Object.entries(v)) {
    const b = await stage(k, () => run(a), res);
    console.log('   frames', b ? b / (64 * 36) : b);
  }
  stopGpu();
  process.exit(0);
})();
