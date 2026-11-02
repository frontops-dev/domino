const { describe, it } = require('node:test')
const assert = require('node:assert')
const fs = require('fs')
const path = require('path')

const { syncCargo, syncIndexJs } = require('../scripts/sync-version.js')

const ROOT = path.join(__dirname, '..')

describe('sync-version.js', () => {
  it('sets the package version in Cargo.toml', () => {
    const cargo = '[package]\nname = "domino"\nversion      = "2.0.6"\n\n[dependencies]\nserde = { version = "1" }\n'

    const updated = syncCargo(cargo, '2.1.0')

    assert.match(updated, /^version {6}= "2\.1\.0"$/m)
    assert.match(updated, /serde = \{ version = "1" \}/)
  })

  it('rewrites every binding version check in index.js', () => {
    const indexJs = [
      "const bindingPackageVersion = require('@front-ops/domino-darwin-arm64/package.json').version",
      "if (bindingPackageVersion !== '0.7.2' && process.env.NAPI_RS_ENFORCE_VERSION_CHECK) {",
      '  throw new Error(`Native binding package version mismatch, expected 0.7.2 but got ${bindingPackageVersion}. You can reinstall dependencies to fix this issue.`)',
      '}',
    ].join('\n')

    const updated = syncIndexJs(indexJs, '2.1.0')

    assert.match(updated, /bindingPackageVersion !== '2\.1\.0'/)
    assert.match(updated, /expected 2\.1\.0 but got/)
    assert.doesNotMatch(updated, /0\.7\.2/)
  })

  it('handles prerelease versions', () => {
    const indexJs = "if (bindingPackageVersion !== '2.1.0' ) { throw new Error(`expected 2.1.0 but got x`) }"

    const updated = syncIndexJs(indexJs, '2.2.0-beta.1')

    assert.match(updated, /!== '2\.2\.0-beta\.1'/)
    assert.match(updated, /expected 2\.2\.0-beta\.1 but got/)
  })

  it('keeps the checked-in index.js in step with package.json', () => {
    const { version } = require(path.join(ROOT, 'package.json'))
    const indexJs = fs.readFileSync(path.join(ROOT, 'index.js'), 'utf8')

    assert.strictEqual(syncIndexJs(indexJs, version), indexJs)
  })
})
