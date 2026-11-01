const { describe, it } = require('node:test')
const assert = require('node:assert')

const { compareVersions, decideRerun } = require('../scripts/release-rerun.js')

describe('release-rerun.js', () => {
  it('is not a re-run when HEAD has no release tag', () => {
    assert.deepStrictEqual(decideRerun([], ['v2.0.0', 'v2.1.0']), { action: 'none' })
  })

  it('reuses the HEAD tag when it is the newest release', () => {
    assert.deepStrictEqual(decideRerun(['v2.1.0'], ['v2.0.0', 'v2.1.0', 'v1.9.9']), {
      action: 'reuse',
      version: '2.1.0',
    })
  })

  it('reports a re-run of an older release as superseded', () => {
    assert.deepStrictEqual(decideRerun(['v2.0.0'], ['v2.0.0', 'v2.1.0']), {
      action: 'superseded',
      version: '2.0.0',
      newest: '2.1.0',
    })
  })

  it('picks the highest of several tags on HEAD', () => {
    assert.deepStrictEqual(decideRerun(['v2.1.0', 'v2.1.1'], ['v2.0.0', 'v2.1.0', 'v2.1.1']), {
      action: 'reuse',
      version: '2.1.1',
    })
  })

  it('ranks a release above its prereleases', () => {
    assert.deepStrictEqual(decideRerun(['v2.1.0-beta.1', 'v2.1.0'], ['v2.1.0', 'v2.1.0-beta.1']), {
      action: 'reuse',
      version: '2.1.0',
    })
    assert.deepStrictEqual(decideRerun(['v2.1.0-beta.1'], ['v2.1.0-beta.1', 'v2.1.0']), {
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
    assert.deepStrictEqual(decideRerun(['v2-latest', 'v2.1.0'], ['v2-latest', 'v2.1.0', 'v9-nightly']), {
      action: 'reuse',
      version: '2.1.0',
    })
    assert.deepStrictEqual(decideRerun(['v2-latest'], ['v2-latest', 'v2.1.0']), { action: 'none' })
  })
})
