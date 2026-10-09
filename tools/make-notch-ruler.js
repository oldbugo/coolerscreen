// Fine ruler for locating the camera hole on the left edge (landscape 2400x1080 coords),
// stored portrait (transpose=1) as the screen expects.
const { execFileSync } = require('child_process');
const ffmpeg = require('ffmpeg-static');

const [W, H] = [2400, 1080];
const out = process.argv[2] || 'notch-ruler.mp4';
const font = 'C\\:/Windows/Fonts/arialbd.ttf';
const f = [];
for (let x = 0; x <= 200; x += 10) {
  const major = x % 50 === 0;
  f.push(`drawbox=x=${x}:y=0:w=${major ? 3 : 2}:h=${H}:color=${major ? 'red' : 'white'}:t=fill`);
}
for (let y = 340; y <= 740; y += 20) {
  const major = y === 540;
  f.push(`drawbox=x=0:y=${y}:w=220:h=${major ? 3 : 2}:color=${major ? 'yellow' : 'cyan'}:t=fill`);
}
for (const x of [50, 100, 150, 200]) {
  f.push(`drawtext=fontfile='${font}':text='${x}':x=${x - 20}:y=120:fontsize=36:fontcolor=red`);
  f.push(`drawtext=fontfile='${font}':text='${x}':x=${x - 20}:y=920:fontsize=36:fontcolor=red`);
}
f.push(`drawtext=fontfile='${font}':text='white lines every 10px  -  cyan every 20px  -  yellow = centre':x=500:y=500:fontsize=48:fontcolor=white`);
f.push('transpose=1');

execFileSync(ffmpeg, [
  '-y', '-f', 'lavfi', '-i', `color=black:s=${W}x${H}:d=1:r=5`,
  '-vf', f.join(','), '-an', '-c:v', 'mjpeg', '-q:v', '4', out,
], { stdio: 'inherit' });
