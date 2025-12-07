#!/usr/bin/env node

const { spawnSync } = require('child_process');
const { platform, arch } = process;

const BIN_NAME = 'domino';

const PLATFORMS = {
  win32: {
    x64: {
      musl: `@front-ops/domino-win32-x64-msvc/${BIN_NAME}.exe`,
      gnu: `@front-ops/domino-win32-x64-msvc/${BIN_NAME}.exe`,
    },
    arm64: {
      musl: `@front-ops/domino-win32-arm64-msvc/${BIN_NAME}.exe`,
      gnu: `@front-ops/domino-win32-arm64-msvc/${BIN_NAME}.exe`,
    },
  },
  darwin: {
    x64: {
      musl: `@front-ops/domino-darwin-x64/${BIN_NAME}`,
      gnu: `@front-ops/domino-darwin-x64/${BIN_NAME}`,
    },
    arm64: {
      musl: `@front-ops/domino-darwin-arm64/${BIN_NAME}`,
      gnu: `@front-ops/domino-darwin-arm64/${BIN_NAME}`,
    },
  },
  linux: {
    x64: {
      musl: `@front-ops/domino-linux-x64-musl/${BIN_NAME}`,
      gnu: `@front-ops/domino-linux-x64-gnu/${BIN_NAME}`,
    },
    arm64: {
      musl: `@front-ops/domino-linux-arm64-musl/${BIN_NAME}`,
      gnu: `@front-ops/domino-linux-arm64-gnu/${BIN_NAME}`,
    },
  },
};

const isMusl = () => {
  if (platform !== 'linux') {
    return false;
  }

  try {
    const { readFileSync } = require('fs');
    return readFileSync('/usr/bin/ldd', 'utf-8').includes('musl');
  } catch {
    try {
      const { execSync } = require('child_process');
      return execSync('ldd --version', { encoding: 'utf8' }).includes('musl');
    } catch {
      return false;
    }
  }
};

let binPath = PLATFORMS[platform]?.[arch]?.[isMusl() ? 'musl' : 'gnu'];

if (binPath) {
  try {
    const result = spawnSync(require.resolve(binPath), process.argv.slice(2), {
      shell: false,
      stdio: 'inherit',
      env: process.env,
    });

    if (result.error) {
      throw result.error;
    }

    process.exitCode = result.status ?? 0;
  } catch (error) {
    console.error(`Error running domino: ${error.message}`);
    process.exitCode = 1;
  }
} else {
  let target = `${platform}-${arch}`;
  if (isMusl()) {
    target = `${target}-musl`;
  }
  console.error(
    `The domino CLI package doesn't ship with prebuilt binaries for your platform (${target}) yet. ` +
      'Please create an issue at https://github.com/frontops-dev/domino/issues for support.',
  );
  process.exitCode = 1;
}
