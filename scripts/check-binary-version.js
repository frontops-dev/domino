#!/usr/bin/env node

// Checks that a built domino binary reports the version about to be published. The
// version is compiled in (`CARGO_PKG_VERSION`, shown by --version and in report
// headers), so a binary built before the release version was set would ship with the
// previous one, as every release up to 2.2.0 did. The publish job runs this before
// claiming the version, so a mismatch stops the release while it is still reversible.
//
// Usage: check-binary-version.js <binary> [expected]   (expected defaults to package.json)

const { spawnSync } = require('child_process')
const path = require('path')

// clap prints `<bin name> <version>` for --version.
function parseVersionOutput(output) {
  const match = /^domino (\S+)\s*$/.exec(output || '')
  return match ? match[1] : null
}

function main({ binary, expected, log = console.log }) {
  if (!expected) {
    log('::error::No expected version to check the binary against')
    return 1
  }
  const result = spawnSync(binary, ['--version'], { encoding: 'utf8' })
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
    console.error('Usage: check-binary-version.js <binary> [expected]')
    process.exit(1)
  }
  const version = expected || require(path.join(__dirname, '..', 'package.json')).version
  process.exit(main({ binary, expected: version }))
}
