const { describe, it, before, after } = require('node:test')
const assert = require('node:assert')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')

const { main, parseVersionOutput } = require('../scripts/check-binary-version.js')

describe('parseVersionOutput', () => {
  it('reads the version clap prints', () => {
    assert.strictEqual(parseVersionOutput('domino 2.2.0\n'), '2.2.0')
    assert.strictEqual(parseVersionOutput('domino 2.2.0-beta.1'), '2.2.0-beta.1')
  })

  it('returns null for anything else', () => {
    assert.strictEqual(parseVersionOutput(''), null)
    assert.strictEqual(parseVersionOutput('error: unexpected argument'), null)
    assert.strictEqual(parseVersionOutput('other 2.2.0'), null)
  })
})

describe('main', () => {
  let dir
  // A stand-in for the release binary: a shell script that answers --version.
  const fakeBinary = (name, script) => {
    const file = path.join(dir, name)
    fs.writeFileSync(file, `#!/bin/sh\n${script}\n`, { mode: 0o755 })
    return file
  }
  const run = (binary, expected) => {
    const lines = []
    const code = main({ binary, expected, log: (line) => lines.push(line) })
    return { code, output: lines.join('\n') }
  }

  before(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'check-binary-version-'))
  })
  after(() => {
    fs.rmSync(dir, { recursive: true, force: true })
  })

  it('passes when the binary reports the release version', () => {
    const { code, output } = run(fakeBinary('current', 'echo "domino 2.2.0"'), '2.2.0')
    assert.strictEqual(code, 0)
    assert.match(output, /reports 2\.2\.0/)
  })

  it('fails when the binary was compiled with an older version', () => {
    // What every release up to 2.2.0 shipped: binaries built before the bump.
    const { code, output } = run(fakeBinary('stale', 'echo "domino 2.1.0"'), '2.2.0')
    assert.strictEqual(code, 1)
    assert.match(output, /::error::.*reports 2\.1\.0.*2\.2\.0/)
  })

  it('compares the whole version, prerelease included', () => {
    const { code } = run(fakeBinary('beta', 'echo "domino 2.2.0-beta.1"'), '2.2.0')
    assert.strictEqual(code, 1)
  })

  it('fails when the binary cannot report a version', () => {
    assert.strictEqual(run(fakeBinary('broken', 'echo boom >&2; exit 3'), '2.2.0').code, 1)
    assert.strictEqual(run(fakeBinary('garbled', 'echo "something else"'), '2.2.0').code, 1)
    assert.strictEqual(run(path.join(dir, 'missing'), '2.2.0').code, 1)
  })

  it('fails without an expected version', () => {
    assert.strictEqual(run(fakeBinary('any', 'echo "domino 2.2.0"'), '').code, 1)
  })
})
