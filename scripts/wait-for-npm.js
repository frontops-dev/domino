#!/usr/bin/env node

// Waits for the platform packages of a just-published release to become visible on
// the npm registry, so the publish job can refresh yarn.lock with a single install.
// Propagation can take minutes per package, and a package that is not visible yet
// looks like a 404 — which is worth waiting out. Any other npm error (auth, network)
// will not fix itself, so it stops the wait at once instead of burning the budget.

const { spawnSync } = require('child_process')
const path = require('path')

const TIMEOUT_MS = 20 * 60 * 1000
const INTERVAL_MS = 15 * 1000

// The platform packages are the root package's `<name>-<platform>` optional
// dependencies, which generate-packages.js has just pinned to the release version.
function platformPackages(manifest) {
  return Object.keys(manifest.optionalDependencies || {}).filter((name) => name.startsWith(`${manifest.name}-`))
}

// `npm view <pkg>@<version> version` prints the version once it is visible, prints
// nothing when the package exists but the version has not propagated, and fails with
// E404 when the package itself is not visible yet.
function classifyView({ status, stdout, stderr }, version) {
  const printed = (stdout || '').trim()
  if (status === 0) {
    if (printed === version) {
      return 'visible'
    }
    return printed === '' ? 'missing' : 'error'
  }
  return /\bE404\b/.test(`${stdout}\n${stderr}`) ? 'missing' : 'error'
}

async function waitForPackages({
  packages,
  version,
  view,
  sleep,
  now,
  log,
  timeoutMs = TIMEOUT_MS,
  intervalMs = INTERVAL_MS,
}) {
  const deadline = now() + timeoutMs
  let pending = packages
  for (;;) {
    const missing = []
    for (const pkg of pending) {
      const result = view(pkg, version)
      const state = classifyView(result, version)
      if (state === 'error') {
        const detail = `${result.stderr || ''}\n${result.stdout || ''}`.trim() || `exit status ${result.status}`
        return { ok: false, reason: 'error', pkg, detail }
      }
      if (state === 'missing') {
        missing.push(pkg)
      }
    }
    if (missing.length === 0) {
      return { ok: true }
    }
    log(`Still waiting for ${missing.length} of ${packages.length} packages at ${version}: ${missing.join(', ')}`)
    if (now() + intervalMs > deadline) {
      return { ok: false, reason: 'timeout', missing }
    }
    pending = missing
    await sleep(intervalMs)
  }
}

// --prefer-online skips npm's local metadata cache, which would otherwise keep
// answering "missing" from the first lookup.
function npmView(pkg, version) {
  const result = spawnSync('npm', ['view', `${pkg}@${version}`, 'version', '--prefer-online'], {
    encoding: 'utf8',
    shell: process.platform === 'win32',
  })
  return {
    status: result.status,
    stdout: result.stdout || '',
    stderr: result.stderr || (result.error ? result.error.message : ''),
  }
}

if (require.main === module) {
  const manifest = require(path.resolve(__dirname, '..', 'package.json'))
  const packages = platformPackages(manifest)
  if (packages.length === 0) {
    console.log(`::error::package.json lists no ${manifest.name}-<platform> optionalDependencies to wait for`)
    process.exit(1)
  }
  waitForPackages({
    packages,
    version: manifest.version,
    view: npmView,
    sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    now: Date.now,
    log: console.log,
  }).then((result) => {
    if (result.ok) {
      console.log(`All ${packages.length} platform packages are visible at ${manifest.version}`)
      return
    }
    if (result.reason === 'error') {
      console.log(
        `::error::npm view ${result.pkg}@${manifest.version} failed with an error that is not propagation delay`,
      )
      console.error(result.detail)
    } else {
      const minutes = TIMEOUT_MS / 60000
      console.log(`::error::Still not visible on npm after ${minutes} minutes: ${result.missing.join(', ')}`)
    }
    process.exit(1)
  })
}

module.exports = { classifyView, platformPackages, waitForPackages }
