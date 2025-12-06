#!/usr/bin/env node

const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const os = require('os');

// Find the domino binary
function findBinary() {
  const platform = os.platform();
  const arch = os.arch();

  const packageRoot = path.join(__dirname, '..');

  // Map platform/arch to package names used by napi-rs
  const platformPackageMap = {
    'darwin': {
      'x64': 'darwin-x64',
      'arm64': 'darwin-arm64',
    },
    'linux': {
      'x64': 'linux-x64',
      'arm64': 'linux-arm64',
    },
    'win32': {
      'x64': 'win32-x64',
    },
  };

  const platformPkg = platformPackageMap[platform]?.[arch];
  const exeExt = platform === 'win32' ? '.exe' : '';

  // Try to find the binary in common locations
  const possiblePaths = [
    // In the package root (most common location after publish)
    path.join(packageRoot, `domino${exeExt}`),
    // In bin directory
    path.join(packageRoot, 'bin', `domino${exeExt}`),
    // Platform-specific npm packages (napi-rs structure)
    ...(platformPkg ? [
      path.join(packageRoot, '..', `@front-ops/domino-${platformPkg}`, `domino${exeExt}`),
      path.join(packageRoot, '..', '..', `@front-ops/domino-${platformPkg}`, `domino${exeExt}`),
    ] : []),
  ];

  for (const binPath of possiblePaths) {
    if (fs.existsSync(binPath)) {
      return binPath;
    }
  }

  throw new Error(
    `Could not find domino binary for ${platform}-${arch}. ` +
    `Tried: ${possiblePaths.join(', ')}. ` +
    `Make sure the package is properly installed and includes the binary.`
  );
}

const binary = findBinary();
const args = process.argv.slice(2);

const child = spawn(binary, args, {
  stdio: 'inherit',
  env: process.env,
});

child.on('error', (err) => {
  console.error(`Error running domino: ${err.message}`);
  process.exit(1);
});

child.on('exit', (code) => {
  process.exit(code || 0);
});

