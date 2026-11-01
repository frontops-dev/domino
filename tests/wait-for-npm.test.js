const { describe, it } = require('node:test')
const assert = require('node:assert')

const { classifyView, platformPackages, waitForPackages } = require('../scripts/wait-for-npm.js')

const MANIFEST = {
  name: '@front-ops/domino',
  version: '2.1.0',
  optionalDependencies: {
    '@front-ops/domino-darwin-arm64': '2.1.0',
    '@front-ops/domino-linux-x64-gnu': '2.1.0',
    '@oxc-node/core-linux-x64-gnu': '0.0.34',
  },
}

const E404 = { status: 1, stdout: '', stderr: 'npm error code E404\nnpm error 404 Not Found' }

// Simulated clock: sleeping advances time instead of waiting.
function fakeClock() {
  let time = 0
  return { now: () => time, sleep: async (ms) => (time += ms) }
}

describe('wait-for-npm.js', () => {
  it('lists only the platform packages of the root package', () => {
    assert.deepStrictEqual(platformPackages(MANIFEST), [
      '@front-ops/domino-darwin-arm64',
      '@front-ops/domino-linux-x64-gnu',
    ])
  })

  it('classifies npm view results', () => {
    assert.strictEqual(classifyView({ status: 0, stdout: '2.1.0\n', stderr: '' }, '2.1.0'), 'visible')
    // The package exists but this version has not propagated yet: npm prints nothing.
    assert.strictEqual(classifyView({ status: 0, stdout: '', stderr: '' }, '2.1.0'), 'missing')
    // The package itself is not visible yet.
    assert.strictEqual(classifyView(E404, '2.1.0'), 'missing')
    assert.strictEqual(classifyView({ status: 1, stdout: '', stderr: 'npm error code E401' }, '2.1.0'), 'error')
    assert.strictEqual(classifyView({ status: 1, stdout: '', stderr: 'npm error code ECONNRESET' }, '2.1.0'), 'error')
    assert.strictEqual(classifyView({ status: 0, stdout: '2.0.9\n', stderr: '' }, '2.1.0'), 'error')
  })

  it('polls until every package reports the version', async () => {
    const clock = fakeClock()
    const visibleAfter = { a: 0, b: 30_000 }
    const viewed = []
    const view = (pkg) => {
      viewed.push([pkg, clock.now()])
      return clock.now() >= visibleAfter[pkg] ? { status: 0, stdout: '2.1.0\n', stderr: '' } : E404
    }
    const result = await waitForPackages({
      packages: ['a', 'b'],
      version: '2.1.0',
      view,
      intervalMs: 15_000,
      timeoutMs: 60_000,
      log: () => {},
      ...clock,
    })
    assert.deepStrictEqual(result, { ok: true })
    // A package is not queried again once it is visible.
    assert.deepStrictEqual(viewed, [
      ['a', 0],
      ['b', 0],
      ['b', 15_000],
      ['b', 30_000],
    ])
  })

  it('logs which packages are still missing each round', async () => {
    const lines = []
    await waitForPackages({
      packages: ['a', 'b'],
      version: '2.1.0',
      view: (pkg) => (pkg === 'a' ? { status: 0, stdout: '2.1.0', stderr: '' } : E404),
      intervalMs: 15_000,
      timeoutMs: 15_000,
      log: (line) => lines.push(line),
      ...fakeClock(),
    })
    assert.deepStrictEqual(lines, [
      'Still waiting for 1 of 2 packages at 2.1.0: b',
      'Still waiting for 1 of 2 packages at 2.1.0: b',
    ])
  })

  it('gives up once the time budget is spent', async () => {
    const clock = fakeClock()
    const result = await waitForPackages({
      packages: ['a'],
      version: '2.1.0',
      view: () => E404,
      intervalMs: 15_000,
      timeoutMs: 60_000,
      log: () => {},
      ...clock,
    })
    assert.deepStrictEqual(result, { ok: false, reason: 'timeout', missing: ['a'] })
    assert.ok(clock.now() <= 60_000)
  })

  it('fails fast on an error that is not propagation', async () => {
    const clock = fakeClock()
    const result = await waitForPackages({
      packages: ['a', 'b'],
      version: '2.1.0',
      view: (pkg) =>
        pkg === 'a' ? E404 : { status: 1, stdout: '', stderr: 'npm error code E401\nUnable to authenticate' },
      intervalMs: 15_000,
      timeoutMs: 60_000,
      log: () => {},
      ...clock,
    })
    assert.strictEqual(result.ok, false)
    assert.strictEqual(result.reason, 'error')
    assert.strictEqual(result.pkg, 'b')
    assert.match(result.detail, /E401/)
    assert.strictEqual(clock.now(), 0)
  })
})
