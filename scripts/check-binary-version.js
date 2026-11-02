#!/usr/bin/env node

// Checks that a built domino binary reports the version about to be published. The
// version is compiled in (`CARGO_PKG_VERSION`, shown by --version and in report
// headers), so a binary built before the release version was set would ship with the
// previous one, as every release up to 2.2.0 did. The publish job runs this before
// claiming the version, so a mismatch stops the release while it is still reversible.
//
// Usage: check-binary-version.js <binary> <expected>

const { spawnSync } = require('child_process')

// --version answers at once; a binary still running after this is hung, and the
// publish job should fail instead of waiting out its time limit.
const TIMEOUT_MS = 30 * 1000

// clap prints `<bin name> <version>` for --version.
function parseVersionOutput(output) {
  const match = /^domino (\S+)\s*$/.exec(output || '')
  return match ? match[1] : null
}

function main({ binary, expected, timeoutMs = TIMEOUT_MS, log = console.log }) {
  if (!expected) {
    log('::error::No expected version to check the binary against')
    return 1
  }
  const result = spawnSync(binary, ['--version'], { encoding: 'utf8', timeout: timeoutMs })
  const reported = result.status === 0 ? parseVersionOutput(result.stdout) : null
  if (reported === null) {
    const detail = result.error ? result.error.message : (result.stderr || result.stdout || '').trim()
    log(`::error::${binary} did not report a version (${detail || `exit ${result.status}`})`)
    return 1
  }
  if (reported !== expected) {
    log(
      `::error::${binary} reports ${reported}, but the release is ${expected}: it was built before the version was set`,
    )
    return 1
  }
  log(`${binary} reports ${reported}`)
  return 0
}

module.exports = { main, parseVersionOutput }

if (require.main === module) {
  const [binary, expected] = process.argv.slice(2)
  if (!binary) {
    console.error('Usage: check-binary-version.js <binary> <expected>')
    process.exit(1)
  }
  // No fallback for a missing version: an empty value from the workflow is a bug to
  // surface, not something to paper over with whatever package.json holds.
  process.exit(main({ binary, expected }))
}
