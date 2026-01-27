#!/usr/bin/env node

const fs = require('fs')
const path = require('path')
const { execSync } = require('child_process')

// Use current working directory as repo root to support running from different directories
const REPO_ROOT = process.cwd()

function execCommand(command, options = {}) {
  console.log(`> ${command}`)
  return execSync(command, {
    cwd: REPO_ROOT,
    encoding: 'utf8',
    stdio: 'inherit',
    ...options,
  })
}

function execCommandCapture(command, options = {}) {
  return execSync(command, {
    cwd: REPO_ROOT,
    encoding: 'utf8',
    ...options,
  }).trim()
}

async function main() {
  // Get environment variables with fallbacks for local development
  const prNumber = process.env.PR_NUMBER || 'local'
  const commitSha = process.env.COMMIT_SHA || execCommandCapture('git rev-parse HEAD').substring(0, 40)
  const repository = process.env.GITHUB_REPOSITORY || (() => {
    try {
      const remoteUrl = execCommandCapture('git config --get remote.origin.url')
      const match = remoteUrl.match(/github\.com[:/](.+?)(?:\.git)?$/)
      return match ? match[1] : 'owner/repo'
    } catch {
      return 'owner/repo'
    }
  })()

  // Get package paths from command line arguments
  const packagePaths = process.argv.slice(2)
  if (packagePaths.length === 0) {
    throw new Error('Usage: publish-preview.js <package-path> [<package-path> ...]')
  }

  const shortSha = commitSha.substring(0, 7)
  const tagName = `pr-${prNumber}-${shortSha}`
  const releaseName = `PR #${prNumber} Preview (${shortSha})`
  const releaseUrl = `https://github.com/${repository}/releases/download/${tagName}`

  // Read package version
  const pkgPath = path.join(REPO_ROOT, 'package.json')
  const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf8'))
  const version = pkg.version

  console.log(`Creating preview release: ${tagName}`)
  console.log(`Version: ${version}`)
  console.log(`Packages to publish: ${packagePaths.join(', ')}`)

  // Pack platform-specific packages
  console.log('\n📦 Packing platform packages...')
  const platformUrls = {}
  const publishedPackageNames = new Set()
  const packages = []

  for (const packagePath of packagePaths) {
    const platformDir = path.resolve(REPO_ROOT, packagePath)

    // Check if this is a directory (platform package) or main package
    if (!fs.existsSync(platformDir)) {
      throw new Error(`Package path not found: ${platformDir}`)
    }

    const stat = fs.statSync(platformDir)
    const isMainPackage = packagePath === '.'

    if (!stat.isDirectory()) {
      throw new Error(`Package path is not a directory: ${platformDir}`)
    }

    // Skip main package for now - we'll handle it after updating optionalDependencies
    if (isMainPackage) {
      continue
    }

    console.log(`\nPacking ${packagePath}...`)

    // Read the platform package.json to get the actual package name
    const platformPkgPath = path.join(platformDir, 'package.json')
    const platformPkg = JSON.parse(fs.readFileSync(platformPkgPath, 'utf8'))
    const packageName = platformPkg.name

    // Pack the package
    const packOutput = execCommandCapture('npm pack --json', { cwd: platformDir })
    const packInfo = JSON.parse(packOutput)
    const tarball = packInfo[0].filename

    // Store tarball info
    const tarballPath = path.join(platformDir, tarball)

    // Store URL for package.json update
    platformUrls[packageName] = `${releaseUrl}/${tarball}`
    publishedPackageNames.add(packageName)

    // Add to packages array for manifest
    packages.push({
      name: packageName,
      tarball: tarball,
      path: tarballPath,
    })

    console.log(`✓ Packed ${tarball} (${packageName})`)
  }

  // Update main package.json optionalDependencies with release URLs
  console.log('\n📝 Updating package.json optionalDependencies...')

  // Only update packages we're publishing, preserve everything else
  const newOptDeps = {}
  for (const [key, value] of Object.entries(pkg.optionalDependencies || {})) {
    if (publishedPackageNames.has(key)) {
      // Replace with GitHub Release URL
      newOptDeps[key] = platformUrls[key]
    } else {
      // Preserve existing dependency as-is
      newOptDeps[key] = value
    }
  }

  pkg.optionalDependencies = newOptDeps

  // Write updated package.json
  fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + '\n')
  console.log('Updated optionalDependencies:')
  console.log(JSON.stringify(newOptDeps, null, 2))

  // Pack main package
  console.log('\n📦 Packing main package...')
  const mainPackOutput = execCommandCapture('npm pack --json')
  const mainPackInfo = JSON.parse(mainPackOutput)
  const mainTarball = mainPackInfo[0].filename
  const mainTarballPath = path.join(REPO_ROOT, mainTarball)

  console.log(`✓ Packed ${mainTarball}`)

  // Print installation instructions
  const installUrl = `${releaseUrl}/${mainTarball}`
  console.log('\n================================================')
  console.log(`✅ Preview release created: ${tagName}`)
  console.log('================================================')
  console.log('')
  console.log('To test this PR, run:')
  console.log('')
  console.log(`  npm install ${installUrl}`)
  console.log('')
  console.log('Direct URL (no auth required):')
  console.log(`  ${installUrl}`)
  console.log('')

  // Generate manifest for CI workflow
  console.log('\n📝 Generating manifest.json...')

  const commentMarker = '<!-- domino-preview-release -->'
  const commentBody = `${commentMarker}
## 📦 Preview Release Available

A preview release has been published for commit ${shortSha}.

### Installation

\`\`\`bash
npm install ${installUrl}
\`\`\`

### Running the preview

\`\`\`bash
npx ${installUrl} affected
\`\`\`

### Details

- **Release**: [${tagName}](https://github.com/${repository}/releases/tag/${tagName})
- **Direct URL**: ${installUrl}`

  const manifest = {
    tagName,
    releaseName,
    shortSha,
    version,
    repository,
    prNumber,
    commitSha,
    releaseNotes: `Preview build for PR #${prNumber} (commit ${shortSha})`,
    packages,
    mainPackage: {
      tarball: mainTarball,
      path: mainTarballPath,
    },
    installUrl,
    commentBody,
  }

  const manifestPath = path.join(REPO_ROOT, 'manifest.json')
  fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + '\n')
  console.log(`✓ Manifest written to ${manifestPath}`)

  console.log('\n✅ Preview release prepared successfully!')
  console.log('📄 manifest.json contains all the information needed for CI to publish the release.')
}

if (require.main === module) {
  main().catch((error) => {
    console.error('Error:', error.message)
    process.exit(1)
  })
}

module.exports = { main }
