// Builds a calibration MJPEG mp4: numbered 50px coloured bands along every edge + frame counter.
const { execFileSync } = require('child_process');
const ffmpeg = require('ffmpeg-static');

const [W, H] = [2400, 1080];
const out = process.argv[2] || 'calibration.mp4';
const rot = process.argv[3] || ''; // e.g. transpose=1 / transpose=2 to store portrait
const font = 'C\\:/Windows/Fonts/arialbd.ttf';
const colors = ['red', 'orange', 'yellow', 'green', 'cyan', 'blue', 'magenta', 'white'];
const f = [];
colors.forEach((c, i) => {
  const o = i * 50;
  f.push(`drawbox=x=${o}:y=0:w=50:h=${H}:color=${c}@0.6:t=fill`);
  f.push(`drawbox=x=${W - o - 50}:y=0:w=50:h=${H}:color=${c}@0.6:t=fill`);
  f.push(`drawbox=x=0:y=${o}:w=${W}:h=50:color=${c}@0.35:t=fill`);
  f.push(`drawbox=x=0:y=${H - o - 50}:w=${W}:h=50:color=${c}@0.35:t=fill`);
  for (const y of [100, 500, 900]) {
    f.push(`drawtext=fontfile='${font}':text='${o}':x=${o + 4}:y=${y}:fontsize=22:fontcolor=black`);
    f.push(`drawtext=fontfile='${font}':text='${o}':x=${W - o - 46}:y=${y}:fontsize=22:fontcolor=black`);
  }
});
f.push(`drawtext=fontfile='${font}':text='LEFT':x=420:y=(h-th)/2:fontsize=60:fontcolor=white`);
f.push(`drawtext=fontfile='${font}':text='RIGHT':x=w-tw-420:y=(h-th)/2:fontsize=60:fontcolor=white`);
f.push(`drawtext=fontfile='${font}':text='TOP':x=(w-tw)/2:y=420:fontsize=60:fontcolor=white`);
f.push(`drawtext=fontfile='${font}':text='frame %{n}':x=(w-tw)/2:y=(h-th)/2:fontsize=90:fontcolor=white`);

execFileSync(ffmpeg, [
  '-y', '-f', 'lavfi', '-i', `color=black:s=${W}x${H}:d=2:r=10`,
  '-vf', f.concat(rot ? [rot] : []).join(','), '-an', '-c:v', 'mjpeg', '-q:v', '12', out,
], { stdio: 'inherit' });



