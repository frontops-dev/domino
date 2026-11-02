const { describe, it } = require('node:test')
const assert = require('node:assert')

const { classifyView, main, platformPackages, waitForPackages } = require('../scripts/wait-for-npm.js')

const MANIFEST = {
  name: '@front-ops/domino',
  version: '2.1.0',
  optionalDependencies: {
    '@front-ops/domino-darwin-arm64': '2.1.0',
    '@front-ops/domino-linux-x64-gnu': '2.1.0',
    // Shares the root name as a prefix but is not one of its platform packages.
    '@front-ops/dominoextras': '1.0.0',
    '@oxc-node/core-linux-x64-gnu': '0.0.34',
  },
}

const E404 = { status: 1, stdout: '', stderr: 'npm error code E404\nnpm error 404 Not Found' }
const VISIBLE = { status: 0, stdout: '2.1.0\n', stderr: '' }
const TIMED_OUT = { status: null, stdout: '', stderr: '', timedOut: true }

// Simulated clock: sleeping advances time instead of waiting. Each sleep still yields
// to the event loop, and a runaway poll loop (say, a missing deadline check) throws
// after MAX_SLEEPS instead of spinning on microtasks until the heap runs out.
const MAX_SLEEPS = 1000
function fakeClock() {
  let time = 0
  let sleeps = 0
  return {
    now: () => time,
    // A concurrent call that finishes at `t` moves the clock there, never backwards.
    advanceTo: (t) => (time = Math.max(time, t)),
    sleep: async (ms) => {
      if (++sleeps > MAX_SLEEPS) {
        throw new Error(`poll loop did not stop after ${MAX_SLEEPS} sleeps`)
      }
      await new Promise(setImmediate)
      time += ms
    },
  }
}

const pick = ({ now, sleep }) => ({ now, sleep })

// A fake `npm view` that takes `delays[pkg]` of simulated time, or times out when that
// exceeds the timeout it is given. Tracks how many calls are in flight at once.
function slowView(clock, delays, { visible = () => true } = {}) {
  const calls = []
  let inFlight = 0
  let maxInFlight = 0
  const view = async (pkg, version, { timeoutMs }) => {
    const start = clock.now()
    calls.push({ pkg, start, timeoutMs })
    maxInFlight = Math.max(maxInFlight, ++inFlight)
    await new Promise(setImmediate)
    inFlight--
    const delay = delays[pkg]
    if (delay > timeoutMs) {
      clock.advanceTo(start + timeoutMs)
      return TIMED_OUT
    }
    clock.advanceTo(start + delay)
    return visible(pkg, start) ? VISIBLE : E404
  }
  return { view, calls, maxInFlight: () => maxInFlight }
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
    // A call that hung past its timeout says nothing about the package: keep polling.
    assert.strictEqual(classifyView(TIMED_OUT, '2.1.0'), 'missing')
  })

  it('queries the packages of a round concurrently', async () => {
    const clock = fakeClock()
    const slow = slowView(clock, { a: 10_000, b: 10_000, c: 10_000 })
    const result = await waitForPackages({
      packages: ['a', 'b', 'c'],
      version: '2.1.0',
      view: slow.view,
      intervalMs: 15_000,
      timeoutMs: 60_000,
      viewTimeoutMs: 30_000,
      log: () => {},
      ...pick(clock),
    })
    assert.deepStrictEqual(result, { ok: true })
    assert.strictEqual(slow.maxInFlight(), 3)
    // Sequential calls would have taken 30 s.
    assert.strictEqual(clock.now(), 10_000)
  })

  it('treats a timed-out view as not visible yet and polls again', async () => {
    const clock = fakeClock()
    const lines = []
    // The first call to `a` hangs; by the second round it answers quickly.
    const delays = { a: 90_000 }
    const view = async (pkg, version, opts) => {
      const result = await slowView(clock, delays).view(pkg, version, opts)
      delays.a = 1_000
      return result
    }
    const result = await waitForPackages({
      packages: ['a'],
      version: '2.1.0',
      view,
      intervalMs: 15_000,
      timeoutMs: 600_000,
      viewTimeoutMs: 60_000,
      log: (line) => lines.push(line),
      ...pick(clock),
    })
    assert.deepStrictEqual(result, { ok: true })
    assert.strictEqual(clock.now(), 60_000 + 15_000 + 1_000)
    assert.ok(
      lines.some((line) => /a@2\.1\.0 timed out after 60s/.test(line)),
      lines.join('\n'),
    )
  })

  it('caps each call by the remaining budget, so the deadline holds', async () => {
    const clock = fakeClock()
    const slow = slowView(clock, { a: Infinity })
    const result = await waitForPackages({
      packages: ['a'],
      version: '2.1.0',
      view: slow.view,
      intervalMs: 15_000,
      timeoutMs: 100_000,
      viewTimeoutMs: 60_000,
      log: () => {},
      ...pick(clock),
    })
    assert.deepStrictEqual(result, { ok: false, reason: 'timeout', missing: ['a'] })
    // 0 s: a full 60 s call; 75 s: only 25 s of budget left for the second one.
    assert.deepStrictEqual(
      slow.calls.map(({ start, timeoutMs }) => [start, timeoutMs]),
      [
        [0, 60_000],
        [75_000, 25_000],
      ],
    )
    assert.strictEqual(clock.now(), 100_000)
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
      now: clock.now,
      sleep: clock.sleep,
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
      timeoutMs: 30_000,
      log: (line) => lines.push(line),
      ...pick(fakeClock()),
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
      now: clock.now,
      sleep: clock.sleep,
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
      now: clock.now,
      sleep: clock.sleep,
    })
    assert.strictEqual(result.ok, false)
    assert.strictEqual(result.reason, 'error')
    assert.strictEqual(result.pkg, 'b')
    assert.match(result.detail, /E401/)
    assert.strictEqual(clock.now(), 0)
  })

  describe('main', () => {
    function run({ manifest = MANIFEST, env = {}, view = () => VISIBLE } = {}) {
      const clock = fakeClock()
      const out = []
      const err = []
      const code = main({
        manifest,
        env,
        view,
        log: (line) => out.push(line),
        error: (line) => err.push(line),
        ...pick(clock),
      })
      return code.then((exitCode) => ({ exitCode, out, err, clock }))
    }

    it('fails when package.json lists no platform packages', async () => {
      const { exitCode, out } = await run({ manifest: { name: '@front-ops/domino', version: '2.1.0' } })
      assert.strictEqual(exitCode, 1)
      assert.match(out.join('\n'), /::error::package\.json lists no @front-ops\/domino-<platform> optionalDependencies/)
    })

    it('exits 0 once every platform package is visible', async () => {
      const viewed = []
      const { exitCode, out } = await run({ view: (pkg) => (viewed.push(pkg), VISIBLE) })
      assert.strictEqual(exitCode, 0)
      assert.deepStrictEqual(viewed, ['@front-ops/domino-darwin-arm64', '@front-ops/domino-linux-x64-gnu'])
      assert.deepStrictEqual(out, ['All 2 platform packages are visible at 2.1.0'])
    })

    it('reports a non-propagation error with its npm output', async () => {
      const { exitCode, out, err } = await run({
        view: () => ({ status: 1, stdout: '', stderr: 'npm error code E401' }),
      })
      assert.strictEqual(exitCode, 1)
      assert.match(out.join('\n'), /::error::npm view @front-ops\/domino-darwin-arm64@2\.1\.0 failed/)
      assert.deepStrictEqual(err, ['npm error code E401'])
    })

    it('gives up after the default 20 minutes', async () => {
      const { exitCode, out, clock } = await run({ view: () => E404 })
      assert.strictEqual(exitCode, 1)
      assert.match(out.at(-1), /^::error::Still not visible on npm after 20 minutes: /)
      assert.ok(clock.now() <= 20 * 60 * 1000)
    })

    it('takes the budget from WAIT_FOR_NPM_TIMEOUT_MS', async () => {
      const { exitCode, out, clock } = await run({ env: { WAIT_FOR_NPM_TIMEOUT_MS: '45000' }, view: () => E404 })
      assert.strictEqual(exitCode, 1)
      assert.match(out.at(-1), /^::error::Still not visible on npm after 45 seconds: /)
      // Rounds at 0, 15 and 30 s; one at 45 s would have no budget left to run in.
      assert.strictEqual(clock.now(), 30_000)
    })

    it('rejects a WAIT_FOR_NPM_TIMEOUT_MS that is not a positive integer', async () => {
      for (const value of ['', '0', '-5', '1.5', 'soon']) {
        const viewed = []
        const { exitCode, out } = await run({
          env: { WAIT_FOR_NPM_TIMEOUT_MS: value },
          view: (pkg) => (viewed.push(pkg), VISIBLE),
        })
        assert.strictEqual(exitCode, 1, `value ${JSON.stringify(value)}`)
        assert.match(out.join('\n'), /::error::WAIT_FOR_NPM_TIMEOUT_MS/)
        assert.deepStrictEqual(viewed, [])
      }
    })
  })
})
