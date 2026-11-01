#!/usr/bin/env node

// Decides whether the publish job is re-running a release. A release tag already on
// HEAD means it is: the newest one is reused instead of bumping from it, which would
// publish a duplicate. An older one has been superseded by a later release, and
// publishing it again would move npm's `latest` dist-tag backwards, so the job must
// stop instead. Prints `<action> [version] [newest]` for the workflow to read.

const { execFileSync } = require('child_process')

const RELEASE_TAG = /^v(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/

function parseTag(tag) {
  const match = RELEASE_TAG.exec(tag)
  if (!match) {
    return null
  }
  const [, major, minor, patch, prerelease] = match
  return { version: tag.slice(1), core: [major, minor, patch].map(Number), prerelease }
}

function compareIdentifiers(a, b) {
  const aNumeric = /^\d+$/.test(a)
  const bNumeric = /^\d+$/.test(b)
  if (aNumeric && bNumeric) {
    return Number(a) - Number(b)
  }
  if (aNumeric !== bNumeric) {
    return aNumeric ? -1 : 1
  }
  return a < b ? -1 : a > b ? 1 : 0
}

// Semver precedence: unlike `sort -V`, a release ranks above its prereleases.
function compareVersions(a, b) {
  const left = parseTag(`v${a}`)
  const right = parseTag(`v${b}`)
  for (let i = 0; i < 3; i++) {
    if (left.core[i] !== right.core[i]) {
      return left.core[i] - right.core[i]
    }
  }
  if (!left.prerelease || !right.prerelease) {
    return (left.prerelease ? -1 : 0) - (right.prerelease ? -1 : 0)
  }
  const leftIds = left.prerelease.split('.')
  const rightIds = right.prerelease.split('.')
  for (let i = 0; i < Math.min(leftIds.length, rightIds.length); i++) {
    const order = compareIdentifiers(leftIds[i], rightIds[i])
    if (order !== 0) {
      return order
    }
  }
  return leftIds.length - rightIds.length
}

function newestVersion(tags) {
  const versions = tags
    .map(parseTag)
    .filter(Boolean)
    .map((tag) => tag.version)
  return versions.sort(compareVersions).pop()
}

function decideRerun(headTags, allTags) {
  const version = newestVersion(headTags)
  if (!version) {
    return { action: 'none' }
  }
  const newest = newestVersion(allTags)
  if (compareVersions(version, newest) < 0) {
    return { action: 'superseded', version, newest }
  }
  return { action: 'reuse', version }
}

function gitTags(...args) {
  return execFileSync('git', ['tag', '--list', 'v[0-9]*', ...args], { encoding: 'utf8' })
    .split('\n')
    .filter(Boolean)
}

if (require.main === module) {
  try {
    const { action, version = '', newest = '' } = decideRerun(gitTags('--points-at', 'HEAD'), gitTags())
    console.log([action, version, newest].join(' ').trim())
  } catch (err) {
    console.error(err.message)
    process.exit(1)
  }
}

module.exports = { compareVersions, decideRerun }
