const path = require('path');
const os = require('os');

const base = process.env.COOLERSCREEN_HOME ||
  path.join(process.env.APPDATA || path.join(os.homedir(), '.config'), 'CoolerScreen');

module.exports = {
  base,
  library: path.join(base, 'library'),
  settings: path.join(base, 'settings.json'),
};
