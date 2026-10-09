#!/usr/bin/env node
require('../src/server').start({ open: !process.argv.includes('--no-open') }).catch(e => {
  console.error('Could not start server: ' + e.message);
  process.exit(1);
});
