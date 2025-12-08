#!/usr/bin/env node

const fs = require('fs')
const path = require('path')
const { execSync } = require('child_process')

const REPO_ROOT = path.resolve(__dirname, '..')

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
  // Get environment variables
  const prNumber = process.env.PR_NUMBER
  const commitSha = process.env.COMMIT_SHA
  const repository = process.env.GITHUB_REPOSITORY

  if (!prNumber || !commitSha || !repository) {
    throw new Error('Missing required environment variables: PR_NUMBER, COMMIT_SHA, GITHUB_REPOSITORY')
  }

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

  // Create GitHub Release
  console.log('\n📦 Creating GitHub Release...')
  execCommand(
    `gh release create "${tagName}" ` +
      `--title "${releaseName}" ` +
      `--notes "Preview build for PR #${prNumber} (commit ${shortSha})" ` +
      `--prerelease ` +
      `--target "${commitSha}"`,
  )

  // Pack and upload platform-specific packages
  console.log('\n📦 Packing and uploading platform packages...')
  const platformUrls = {}
  const publishedPackageNames = new Set()

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

    // Upload to release
    const tarballPath = path.join(platformDir, tarball)
    execCommand(`gh release upload "${tagName}" "${tarballPath}"`)

    // Store URL for package.json update
    platformUrls[packageName] = `${releaseUrl}/${tarball}`
    publishedPackageNames.add(packageName)

    console.log(`✓ Uploaded ${tarball} (${packageName})`)
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

  // Pack and upload main package
  console.log('\n📦 Packing and uploading main package...')
  const mainPackOutput = execCommandCapture('npm pack --json')
  const mainPackInfo = JSON.parse(mainPackOutput)
  const mainTarball = mainPackInfo[0].filename

  execCommand(`gh release upload "${tagName}" "${mainTarball}"`)
  console.log(`✓ Uploaded ${mainTarball}`)

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
}

if (require.main === module) {
  main().catch((error) => {
    console.error('Error:', error.message)
    process.exit(1)
  })
}

module.exports = { main }
