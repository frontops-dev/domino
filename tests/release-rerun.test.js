const { describe, it, before, after } = require('node:test')
const assert = require('node:assert')
const { execFileSync } = require('child_process')
const fs = require('fs')
const os = require('os')
const path = require('path')

const { compareVersions, decideRerun } = require('../scripts/release-rerun.js')

const SCRIPT = path.join(__dirname, '..', 'scripts', 'release-rerun.js')
const always = () => true
const never = () => false

describe('release-rerun.js', () => {
  it('is not a re-run when HEAD has no release tag', () => {
    assert.deepStrictEqual(decideRerun([], ['v2.0.0', 'v2.1.0'], always), { action: 'none' })
  })

  it('reuses the HEAD tag when it is the newest release', () => {
    assert.deepStrictEqual(decideRerun(['v2.1.0'], ['v2.0.0', 'v2.1.0', 'v1.9.9'], always), {
      action: 'reuse',
      version: '2.1.0',
    })
  })

  it('reports a re-run of an older release as superseded', () => {
    assert.deepStrictEqual(decideRerun(['v2.0.0'], ['v2.0.0', 'v2.1.0'], always), {
      action: 'superseded',
      version: '2.0.0',
      newest: '2.1.0',
    })
  })

  it('picks the highest of several tags on HEAD', () => {
    assert.deepStrictEqual(decideRerun(['v2.1.0', 'v2.1.1'], ['v2.0.0', 'v2.1.0', 'v2.1.1'], always), {
      action: 'reuse',
      version: '2.1.1',
    })
  })

  it('ranks a release above its prereleases', () => {
    assert.deepStrictEqual(decideRerun(['v2.1.0-beta.1', 'v2.1.0'], ['v2.1.0', 'v2.1.0-beta.1'], always), {
      action: 'reuse',
      version: '2.1.0',
    })
    assert.deepStrictEqual(decideRerun(['v2.1.0-beta.1'], ['v2.1.0-beta.1', 'v2.1.0'], always), {
      action: 'superseded',
      version: '2.1.0-beta.1',
      newest: '2.1.0',
    })
  })

  it('compares versions numerically, not as text', () => {
    assert.ok(compareVersions('2.10.0', '2.9.0') > 0)
    assert.ok(compareVersions('2.1.0', '2.1.0-beta.1') > 0)
    assert.ok(compareVersions('2.1.0-beta.10', '2.1.0-beta.9') > 0)
    assert.ok(compareVersions('2.1.0-beta', '2.1.0-alpha') > 0)
    assert.ok(compareVersions('2.1.0-beta.1', '2.1.0-beta') > 0)
    assert.ok(compareVersions('2.1.0-1', '2.1.0-alpha') < 0)
    assert.strictEqual(compareVersions('2.1.0', '2.1.0'), 0)
  })

  it('ignores tags that are not versions', () => {
    assert.deepStrictEqual(decideRerun(['v2-latest', 'v2.1.0'], ['v2-latest', 'v2.1.0', 'v9-nightly'], always), {
      action: 'reuse',
      version: '2.1.0',
    })
    assert.deepStrictEqual(decideRerun(['v2-latest'], ['v2-latest', 'v2.1.0'], always), { action: 'none' })
  })
  it('reports HEAD as superseded when the newest release is not an ancestor', () => {
    assert.deepStrictEqual(decideRerun([], ['v2.0.5', 'v3.0.0'], never), { action: 'superseded', newest: '3.0.0' })
  })

  it('asks about the ancestry of the newest release only', () => {
    const asked = []
    decideRerun([], ['v2.0.5', 'v3.0.0', 'v2.1.0'], (tag) => asked.push(tag) && true)
    assert.deepStrictEqual(asked, ['v3.0.0'])
  })

  it('ignores prereleases when HEAD is not on a prerelease', () => {
    assert.deepStrictEqual(decideRerun(['v2.1.0'], ['v2.1.0', 'v2.2.0-beta.1'], always), {
      action: 'reuse',
      version: '2.1.0',
    })
    assert.deepStrictEqual(
      decideRerun([], ['v2.1.0', 'v2.2.0-beta.1'], (tag) => tag === 'v2.1.0'),
      {
        action: 'none',
      },
    )
  })

  it('counts prereleases when HEAD is on a prerelease', () => {
    assert.deepStrictEqual(decideRerun(['v2.2.0-beta.1'], ['v2.1.0', 'v2.2.0-beta.1'], always), {
      action: 'reuse',
      version: '2.2.0-beta.1',
    })
    assert.deepStrictEqual(decideRerun(['v2.2.0-beta.1'], ['v2.1.0', 'v2.2.0-beta.1', 'v2.2.0-beta.2'], always), {
      action: 'superseded',
      version: '2.2.0-beta.1',
      newest: '2.2.0-beta.2',
    })
  })

  it('is not a re-run when the repository has no release tags', () => {
    assert.deepStrictEqual(decideRerun([], [], never), { action: 'none' })
  })
})

// Runs the script against a real repository, so the ancestry check and the printed
// line the workflow reads are covered end to end.
describe('release-rerun.js in a git repository', () => {
  let dir
  const commits = {}

  const git = (...args) => execFileSync('git', args, { cwd: dir, encoding: 'utf8' }).trim()
  const commit = (name, subject) => {
    git('commit', '-q', '--no-verify', '--allow-empty', '-m', subject)
    commits[name] = git('rev-parse', 'HEAD')
  }
  const runAt = (name) => {
    git('checkout', '-q', '--detach', commits[name])
    return execFileSync('node', [SCRIPT], { cwd: dir, encoding: 'utf8' }).trim()
  }

  // A (v2.0.5) -> X (feat, publish cancelled) -> Y (feat!, v3.0.0) -> V (3.0.0) -> Z
  //                                                \-> B (v3.1.0-beta.1, side branch)
  before(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'release-rerun-'))
    git('init', '-q', '-b', 'main')
    git('config', 'user.name', 'Test')
    git('config', 'user.email', 'test@example.com')
    git('config', 'commit.gpgsign', 'false')
    git('config', 'tag.gpgsign', 'false')
    commit('A', 'fix: a')
    git('tag', 'v2.0.5')
    commit('X', 'feat: x')
    commit('Y', 'feat!: y')
    git('tag', 'v3.0.0')
    git('checkout', '-q', '-b', 'beta')
    commit('B', 'feat: b')
    git('tag', 'v3.1.0-beta.1')
    git('checkout', '-q', 'main')
    commit('V', '3.0.0')
    commit('Z', 'fix: z')
  })

  after(() => {
    fs.rmSync(dir, { recursive: true, force: true })
  })

  it('bumps normally on a commit after the newest release', () => {
    assert.strictEqual(runAt('Z'), 'none')
  })

  it('bumps normally on the version commit, whose tag is on its parent', () => {
    assert.strictEqual(runAt('V'), 'none')
  })

  it('skips a stale untagged run behind a newer release', () => {
    assert.strictEqual(runAt('X'), 'superseded - 3.0.0')
  })

  it('reuses the tag of the newest release', () => {
    assert.strictEqual(runAt('Y'), 'reuse 3.0.0')
  })

  it('skips a re-run of an older tagged release', () => {
    assert.strictEqual(runAt('A'), 'superseded 2.0.5 3.0.0')
  })

  it('reuses a prerelease tag on HEAD', () => {
    assert.strictEqual(runAt('B'), 'reuse 3.1.0-beta.1')
  })
})
