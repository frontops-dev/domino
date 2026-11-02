#!/usr/bin/env node

// Waits for the platform packages of a just-published release to become visible on
// the npm registry, so the publish job can refresh yarn.lock with a single install.
// Propagation can take minutes per package, and a package that is not visible yet
// looks like a 404 — which is worth waiting out. Any other npm error (auth, network)
// will not fix itself, so it stops the wait at once instead of burning the budget.

const { spawn } = require('child_process')
const path = require('path')

const TIMEOUT_MS = 20 * 60 * 1000
const INTERVAL_MS = 15 * 1000
// One npm view normally answers in a few seconds, but a slow registry or network can
// hang it for minutes. A call that hangs this long is killed and counts as "not
// visible yet" rather than as an error: a hang says nothing about the package, and
// the overall budget still bounds the wait if it keeps happening.
const VIEW_TIMEOUT_MS = 2 * 60 * 1000

// The platform packages are the root package's `<name>-<platform>` optional
// dependencies, which generate-packages.js has just pinned to the release version.
function platformPackages(manifest) {
  return Object.keys(manifest.optionalDependencies || {}).filter((name) => name.startsWith(`${manifest.name}-`))
}

// `npm view <pkg>@<version> version` prints the version once it is visible. Until then
// it fails with E404, both when the package itself is not visible and when it exists
// without that version (npm 11 reports "No match found for version"); older npm
// versions print nothing and exit 0 for the latter, so empty output also counts. A
// call that timed out counts as missing too (see VIEW_TIMEOUT_MS).
function classifyView({ status, stdout, stderr, timedOut }, version) {
  if (timedOut) {
    return 'missing'
  }
  const printed = (stdout || '').trim()
  if (status === 0) {
    if (printed === version) {
      return 'visible'
    }
    return printed === '' ? 'missing' : 'error'
  }
  return /\bE404\b/.test(`${stdout}\n${stderr}`) ? 'missing' : 'error'
}

// Each round queries the pending packages concurrently, and each call's timeout is
// capped by what is left of the budget, so a round never runs past the deadline. A
// new round starts only if the deadline is still ahead after the interval.
async function waitForPackages({
  packages,
  version,
  view,
  sleep,
  now,
  log,
  timeoutMs = TIMEOUT_MS,
  intervalMs = INTERVAL_MS,
  viewTimeoutMs = VIEW_TIMEOUT_MS,
}) {
  const deadline = now() + timeoutMs
  let pending = packages
  for (;;) {
    const callTimeoutMs = Math.min(viewTimeoutMs, deadline - now())
    const results = await Promise.all(pending.map((pkg) => view(pkg, version, { timeoutMs: callTimeoutMs })))
    const missing = []
    for (const [i, pkg] of pending.entries()) {
      const result = results[i]
      const state = classifyView(result, version)
      if (state === 'error') {
        const detail = `${result.stderr || ''}\n${result.stdout || ''}`.trim() || `exit status ${result.status}`
        return { ok: false, reason: 'error', pkg, detail }
      }
      if (result.timedOut) {
        log(`npm view ${pkg}@${version} timed out after ${callTimeoutMs / 1000}s; treating it as not visible yet`)
      }
      if (state === 'missing') {
        missing.push(pkg)
      }
    }
    if (missing.length === 0) {
      return { ok: true }
    }
    log(`Still waiting for ${missing.length} of ${packages.length} packages at ${version}: ${missing.join(', ')}`)
    if (now() + intervalMs >= deadline) {
      return { ok: false, reason: 'timeout', missing }
    }
    pending = missing
    await sleep(intervalMs)
  }
}

// --prefer-online skips npm's local metadata cache, which would otherwise keep
// answering "missing" from the first lookup. On timeout the result is returned at
// once, without waiting for the killed process to exit.
function npmView(pkg, version, { timeoutMs }) {
  return new Promise((resolve) => {
    let stdout = ''
    let stderr = ''
    let settled = false
    const finish = (result) => {
      if (!settled) {
        settled = true
        clearTimeout(timer)
        resolve({ stdout, stderr, ...result })
      }
    }
    const child = spawn('npm', ['view', `${pkg}@${version}`, 'version', '--prefer-online'], {
      shell: process.platform === 'win32',
    })
    const timer = setTimeout(() => {
      child.kill()
      finish({ status: null, timedOut: true })
    }, timeoutMs)
    child.stdout.setEncoding('utf8').on('data', (chunk) => (stdout += chunk))
    child.stderr.setEncoding('utf8').on('data', (chunk) => (stderr += chunk))
    child.on('error', (err) => finish({ status: null, stderr: stderr || err.message }))
    child.on('close', (status) => finish({ status }))
  })
}

function describeBudget(ms) {
  return ms % 60000 === 0 ? `${ms / 60000} minutes` : `${ms / 1000} seconds`
}

// WAIT_FOR_NPM_TIMEOUT_MS overrides the overall budget, for exercising the step
// without waiting 20 minutes.
async function main({ manifest, env, view, sleep, now, log, error }) {
  const packages = platformPackages(manifest)
  if (packages.length === 0) {
    log(`::error::package.json lists no ${manifest.name}-<platform> optionalDependencies to wait for`)
    return 1
  }
  let timeoutMs = TIMEOUT_MS
  const override = env.WAIT_FOR_NPM_TIMEOUT_MS
  if (override !== undefined) {
    if (!/^[1-9][0-9]*$/.test(override)) {
      log(`::error::WAIT_FOR_NPM_TIMEOUT_MS must be a positive whole number of milliseconds, got "${override}"`)
      return 1
    }
    timeoutMs = Number(override)
  }
  const result = await waitForPackages({ packages, version: manifest.version, view, sleep, now, log, timeoutMs })
  if (result.ok) {
    log(`All ${packages.length} platform packages are visible at ${manifest.version}`)
    return 0
  }
  if (result.reason === 'error') {
    log(`::error::npm view ${result.pkg}@${manifest.version} failed with an error that is not propagation delay`)
    error(result.detail)
  } else {
    log(`::error::Still not visible on npm after ${describeBudget(timeoutMs)}: ${result.missing.join(', ')}`)
  }
  return 1
}

if (require.main === module) {
  main({
    manifest: require(path.resolve(__dirname, '..', 'package.json')),
    env: process.env,
    view: npmView,
    sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    now: Date.now,
    log: console.log,
    error: console.error,
  }).then((code) => {
    process.exitCode = code
  })
}

module.exports = { classifyView, main, platformPackages, waitForPackages }
