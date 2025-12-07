#!/usr/bin/env node

const fs = require('fs');
const path = require('path');

const REPO_ROOT = path.resolve(__dirname, '..');
const NPM_DIR = path.join(REPO_ROOT, 'npm');
const ARTIFACTS_DIR = path.join(REPO_ROOT, 'artifacts');
const BIN_NAME = 'domino';
const PKG_NAME = '@front-ops/domino';

const TARGET_TO_PACKAGE = {
  'x86_64-apple-darwin': {
    name: 'darwin-x64',
    binary: 'domino',
    ext: '',
  },
  'aarch64-apple-darwin': {
    name: 'darwin-arm64',
    binary: 'domino',
    ext: '',
  },
  'x86_64-pc-windows-msvc': {
    name: 'win32-x64-msvc',
    binary: 'domino.exe',
    ext: '.exe',
  },
  'x86_64-unknown-linux-gnu': {
    name: 'linux-x64-gnu',
    binary: 'domino',
    ext: '',
  },
};

function readRootPackageJson() {
  const pkgPath = path.join(REPO_ROOT, 'package.json');
  return JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
}

function findArtifact(target) {
  const artifactDir = path.join(ARTIFACTS_DIR, `bindings-${target}`);
  if (!fs.existsSync(artifactDir)) {
    return null;
  }

  const files = fs.readdirSync(artifactDir);
  const nodeFile = files.find((file) => file.endsWith('.node'));
  const binaryFile = files.find((file) => file.startsWith(`${BIN_NAME}-${target}`));

  return {
    dir: artifactDir,
    node: nodeFile ? path.join(artifactDir, nodeFile) : null,
    binary: binaryFile ? path.join(artifactDir, binaryFile) : null,
  };
}

function main() {
  const rootPkg = readRootPackageJson();
  const version = rootPkg.version;

  if (!fs.existsSync(NPM_DIR)) {
    console.error(`Error: npm directory not found at ${NPM_DIR}`);
    console.error('Please run "yarn artifacts" first to copy .node files');
    process.exit(1);
  }

  for (const [target, targetInfo] of Object.entries(TARGET_TO_PACKAGE)) {
    const artifact = findArtifact(target);
    if (!artifact) {
      console.warn(`Warning: Artifact not found for ${target}, skipping`);
      continue;
    }

    const packageRoot = path.join(NPM_DIR, targetInfo.name);
    if (!fs.existsSync(packageRoot)) {
      console.warn(`Warning: Package directory not found: ${packageRoot}, skipping`);
      continue;
    }

    const pkgJsonPath = path.join(packageRoot, 'package.json');
    if (!fs.existsSync(pkgJsonPath)) {
      console.warn(`Warning: package.json not found at ${pkgJsonPath}, skipping`);
      continue;
    }

    if (!artifact.binary) {
      console.warn(`Warning: Binary not found in artifact for ${target}, skipping`);
      continue;
    }

    const binaryDest = path.join(packageRoot, targetInfo.binary);
    fs.copyFileSync(artifact.binary, binaryDest);
    if (!targetInfo.ext) {
      fs.chmodSync(binaryDest, 0o755);
    }
    console.log(`✓ Copied ${targetInfo.binary} to ${targetInfo.name}`);

    const pkg = JSON.parse(fs.readFileSync(pkgJsonPath, 'utf8'));

    if (!pkg.files) {
      pkg.files = [];
    }
    if (!pkg.files.includes(targetInfo.binary)) {
      pkg.files.push(targetInfo.binary);
    }

    if (!pkg.publishConfig) {
      pkg.publishConfig = {};
    }
    if (!pkg.publishConfig.executableFiles) {
      pkg.publishConfig.executableFiles = [];
    }
    if (!pkg.publishConfig.executableFiles.includes(targetInfo.binary)) {
      pkg.publishConfig.executableFiles.push(targetInfo.binary);
    }

    fs.writeFileSync(pkgJsonPath, JSON.stringify(pkg, null, 2) + '\n');
    console.log(`✓ Updated package.json for ${targetInfo.name}`);
  }
}

if (require.main === module) {
  main();
}

module.exports = { main };
