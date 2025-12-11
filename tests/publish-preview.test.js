const { describe, it, before, after } = require('node:test')
const assert = require('node:assert')
const fs = require('fs')
const path = require('path')
const { execSync } = require('child_process')

const FIXTURES_DIR = path.join(__dirname, 'fixtures', 'test-repo')
const SCRIPT_PATH = path.join(__dirname, '..', 'scripts', 'publish-preview.js')

describe('publish-preview.js', () => {
  let originalCwd

  before(() => {
    originalCwd = process.cwd()
  })

  after(() => {
    process.chdir(originalCwd)
  })

  it('should generate valid manifest.json', () => {
    // Clean up any existing manifest and tarballs
    const manifestPath = path.join(FIXTURES_DIR, 'manifest.json')
    if (fs.existsSync(manifestPath)) {
      fs.unlinkSync(manifestPath)
    }
    const tarballs = fs.readdirSync(FIXTURES_DIR).filter((f) => f.endsWith('.tgz'))
    tarballs.forEach((t) => fs.unlinkSync(path.join(FIXTURES_DIR, t)))

    // Run the script from fixture directory
    try {
      execSync(`node "${SCRIPT_PATH}" ./npm/test-platform .`, {
        cwd: FIXTURES_DIR,
        env: {
          ...process.env,
          PR_NUMBER: '123',
          COMMIT_SHA: 'abc1234567890123456789012345678901234567',
          GITHUB_REPOSITORY: 'test/repo',
        },
        stdio: 'pipe',
      })
    } catch (error) {
      console.error('Script failed:', error.stdout?.toString(), error.stderr?.toString())
      throw error
    }

    // Verify manifest.json was created
    assert.ok(fs.existsSync(manifestPath), 'manifest.json should exist')

    const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'))

    // Verify manifest structure
    assert.strictEqual(manifest.tagName, 'pr-123-abc1234', 'tagName should match pattern')
    assert.strictEqual(manifest.prNumber, '123', 'prNumber should match')
    assert.strictEqual(
      manifest.commitSha,
      'abc1234567890123456789012345678901234567',
      'commitSha should match',
    )
    assert.strictEqual(manifest.repository, 'test/repo', 'repository should match')
    assert.strictEqual(manifest.version, '0.1.0', 'version should match package.json')
    assert.ok(Array.isArray(manifest.packages), 'packages should be an array')
    assert.ok(manifest.packages.length > 0, 'packages array should not be empty')
    assert.ok(manifest.mainPackage, 'mainPackage should exist')
    assert.ok(manifest.mainPackage.tarball, 'mainPackage.tarball should exist')
    assert.ok(manifest.commentBody, 'commentBody should exist')
    assert.ok(manifest.commentBody.includes('Preview Release Available'), 'commentBody should contain expected text')
  })

  it('should update package.json optionalDependencies with release URLs', () => {
    const manifestPath = path.join(FIXTURES_DIR, 'manifest.json')
    const packageJsonPath = path.join(FIXTURES_DIR, 'package.json')

    // Clean up
    if (fs.existsSync(manifestPath)) {
      fs.unlinkSync(manifestPath)
    }

    // Reset package.json to original state
    const pkg = {
      name: 'domino-test',
      version: '0.1.0',
      description: 'Test package for publish-preview.js',
      main: 'index.js',
      optionalDependencies: {
        '@domino/test-platform': '^0.1.0',
      },
    }
    fs.writeFileSync(packageJsonPath, JSON.stringify(pkg, null, 2) + '\n')

    // Run the script
    execSync(`node "${SCRIPT_PATH}" ./npm/test-platform .`, {
      cwd: FIXTURES_DIR,
      env: {
        ...process.env,
        PR_NUMBER: '456',
        COMMIT_SHA: 'def4567890123456789012345678901234567890',
        GITHUB_REPOSITORY: 'test/repo',
      },
      stdio: 'pipe',
    })

    // Read updated package.json
    const updatedPkg = JSON.parse(fs.readFileSync(packageJsonPath, 'utf8'))

    // Verify optionalDependencies was updated with GitHub release URL
    const depValue = updatedPkg.optionalDependencies['@domino/test-platform']
    assert.ok(depValue.startsWith('https://github.com/'), 'optionalDependency should be a GitHub URL')
    assert.ok(depValue.includes('releases/download'), 'optionalDependency should point to release download')
    assert.ok(depValue.includes('pr-456-def4567'), 'optionalDependency should include tag name')
  })

  it('should create tarballs for all packages', () => {
    const manifestPath = path.join(FIXTURES_DIR, 'manifest.json')

    // Clean up tarballs
    const tarballs = fs.readdirSync(FIXTURES_DIR).filter((f) => f.endsWith('.tgz'))
    tarballs.forEach((t) => fs.unlinkSync(path.join(FIXTURES_DIR, t)))

    // Run the script
    execSync(`node "${SCRIPT_PATH}" ./npm/test-platform .`, {
      cwd: FIXTURES_DIR,
      env: {
        ...process.env,
        PR_NUMBER: '789',
        COMMIT_SHA: 'ghi7890123456789012345678901234567890123',
        GITHUB_REPOSITORY: 'test/repo',
      },
      stdio: 'pipe',
    })

    // Verify tarballs exist
    const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'))

    // Check platform package tarball
    assert.ok(manifest.packages.length > 0, 'should have platform packages')
    const platformPkg = manifest.packages[0]
    assert.ok(fs.existsSync(platformPkg.path), `platform tarball should exist at ${platformPkg.path}`)

    // Check main package tarball
    assert.ok(fs.existsSync(manifest.mainPackage.path), `main tarball should exist at ${manifest.mainPackage.path}`)
  })

  it('should work with local defaults when env vars are missing', () => {
    const manifestPath = path.join(FIXTURES_DIR, 'manifest.json')

    // Clean up
    if (fs.existsSync(manifestPath)) {
      fs.unlinkSync(manifestPath)
    }

    // Run without environment variables (will use git commands for defaults)
    try {
      execSync(`node "${SCRIPT_PATH}" ./npm/test-platform .`, {
        cwd: FIXTURES_DIR,
        stdio: 'pipe',
      })
    } catch (error) {
      // It's okay if this fails in some environments (no git repo in fixture)
      // The important thing is it doesn't throw "Missing required environment variables"
      const errorMsg = error.stderr?.toString() || error.message
      assert.ok(
        !errorMsg.includes('Missing required environment variables'),
        'should not require environment variables',
      )
      return
    }

    // If it succeeded, verify manifest was created
    assert.ok(fs.existsSync(manifestPath), 'manifest.json should exist even without env vars')
  })

  it('should handle error for missing package path', () => {
    // Try to run with non-existent package path
    assert.throws(
      () => {
        execSync(`node "${SCRIPT_PATH}" ./npm/nonexistent .`, {
          cwd: FIXTURES_DIR,
          env: {
            ...process.env,
            PR_NUMBER: '999',
            COMMIT_SHA: 'xyz9999999999999999999999999999999999999',
            GITHUB_REPOSITORY: 'test/repo',
          },
          stdio: 'pipe',
        })
      },
      (error) => {
        const errorMsg = error.stderr?.toString() || error.message
        return errorMsg.includes('Package path not found')
      },
      'should throw error for missing package path',
    )
  })
})
