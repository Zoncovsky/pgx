#!/usr/bin/env node
'use strict';
require('../lib/pgxdb.js')
  .main()
  .then((code) => {
    process.exitCode = code;
  })
  .catch((e) => {
    process.stderr.write(`pgxdb: ${e && e.stack ? e.stack : e}\n`);
    process.exitCode = 1;
  });
