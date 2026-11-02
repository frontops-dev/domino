const { describe, it, before, after } = require('node:test')
const assert = require('node:assert')
const { spawnSync } = require('node:child_process')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')

const { main, parseVersionOutput } = require('../scripts/check-binary-version.js')

describe('parseVersionOutput', () => {
  it('reads the version clap prints', () => {
    assert.strictEqual(parseVersionOutput('domino 2.2.0\n'), '2.2.0')
    assert.strictEqual(parseVersionOutput('domino 2.2.0-beta.1'), '2.2.0-beta.1')
  })

  it('accepts a Windows line ending', () => {
    assert.strictEqual(parseVersionOutput('domino 2.2.0\r\n'), '2.2.0')
  })

  it('returns null for anything else', () => {
    assert.strictEqual(parseVersionOutput(''), null)
    assert.strictEqual(parseVersionOutput('error: unexpected argument'), null)
    assert.strictEqual(parseVersionOutput('other 2.2.0'), null)
    // A version line followed by anything else is not what clap prints for --version.
    assert.strictEqual(parseVersionOutput('domino 2.2.0\nwarning: something'), null)
  })
})

// The stand-in binaries are shell scripts, which Windows cannot execute directly. The
// check itself only runs on the Linux publish runner, where these suites do run.
const shellScripts = process.platform === 'win32' && 'needs executable shell scripts'

describe('main', { skip: shellScripts }, () => {
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

  it('gives up on a binary that hangs', () => {
    const lines = []
    const started = Date.now()
    const code = main({
      binary: fakeBinary('hang', 'sleep 10'),
      expected: '2.2.0',
      timeoutMs: 200,
      log: (line) => lines.push(line),
    })
    assert.strictEqual(code, 1)
    assert.match(lines.join('\n'), /::error::/)
    assert.ok(Date.now() - started < 5000, 'waited for the binary instead of timing out')
  })
})

describe('command line', { skip: shellScripts }, () => {
  const script = path.join(__dirname, '..', 'scripts', 'check-binary-version.js')
  let dir
  before(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'check-binary-version-cli-'))
    fs.writeFileSync(path.join(dir, 'bin'), '#!/bin/sh\necho "domino 2.2.0"\n', { mode: 0o755 })
  })
  after(() => {
    fs.rmSync(dir, { recursive: true, force: true })
  })
  const cli = (...args) => spawnSync(process.execPath, [script, ...args], { encoding: 'utf8' })

  it('exits 0 on a match and 1 on a mismatch', () => {
    assert.strictEqual(cli(path.join(dir, 'bin'), '2.2.0').status, 0)
    assert.strictEqual(cli(path.join(dir, 'bin'), '2.2.1').status, 1)
  })

  it('requires the expected version rather than guessing it', () => {
    // An empty value from the workflow must fail, not fall back to package.json.
    assert.strictEqual(cli(path.join(dir, 'bin'), '').status, 1)
    assert.strictEqual(cli(path.join(dir, 'bin')).status, 1)
    assert.strictEqual(cli().status, 1)
  })
})
