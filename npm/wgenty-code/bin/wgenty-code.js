#!/usr/bin/env node
'use strict';

// Launcher: locates the prebuilt platform binary installed as an
// optionalDependency and spawns it, forwarding stdio + exit code.

const { dirname, join } = require('node:path');
const { spawn } = require('node:child_process');

const { platform, arch } = process;

// Maps `process.platform`-`process.arch` to the platform subpackage name.
const PLATFORM_SUPPORT = {
  'linux-x64': 'wgenty-code-linux-x64',
  'linux-arm64': 'wgenty-code-linux-arm64',
  'darwin-x64': 'wgenty-code-darwin-x64',
  'darwin-arm64': 'wgenty-code-darwin-arm64',
  'win32-x64': 'wgenty-code-win32-x64',
};

const key = `${platform}-${arch}`;
const pkg = PLATFORM_SUPPORT[key];

if (!pkg) {
  console.error(`wgenty-code: unsupported platform ${key}`);
  console.error(
    'Supported: linux-x64, linux-arm64, darwin-x64, darwin-arm64, win32-x64'
  );
  process.exit(1);
}

const exe = platform === 'win32' ? 'wgenty-code.exe' : 'wgenty-code';

let pkgDir;
try {
  // package.json is always resolvable for an installed package.
  pkgDir = dirname(require.resolve(`${pkg}/package.json`));
} catch {
  console.error(`wgenty-code: platform package "${pkg}" is not installed.`);
  console.error(
    'Try reinstalling wgenty-code (npm install -g wgenty-code).'
  );
  process.exit(1);
}

const binPath = join(pkgDir, exe);

const child = spawn(binPath, process.argv.slice(2), { stdio: 'inherit' });

child.on('error', (err) => {
  console.error(`wgenty-code: failed to launch binary: ${err.message}`);
  process.exit(1);
});

child.on('exit', (code, signal) => {
  if (signal) {
    try {
      process.kill(process.pid, signal);
    } catch {
      process.exit(1);
    }
  }
  process.exit(code ?? 1);
});
