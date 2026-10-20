#!/usr/bin/env node

// Copies the package.json version into the files that repeat it: Cargo.toml, and
// the native-binding version checks napi generated into index.js (enforced when
// NAPI_RS_ENFORCE_VERSION_CHECK is set).

const fs = require('fs')
const path = require('path')

const CARGO_VERSION = /^(version\s*=\s*)".*"/m
const VERSION = String.raw`\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?`
const BINDING_CHECK = new RegExp(String.raw`(bindingPackageVersion !== ')${VERSION}(')`, 'g')
const BINDING_MESSAGE = new RegExp(String.raw`(expected )${VERSION}( but got)`, 'g')

function syncCargo(cargo, version) {
  if (!CARGO_VERSION.test(cargo)) {
    throw new Error('No version field found in Cargo.toml')
  }
  return cargo.replace(CARGO_VERSION, `$1"${version}"`)
}

function syncIndexJs(indexJs, version) {
  if (!new RegExp(BINDING_CHECK.source).test(indexJs)) {
    throw new Error('No native binding version check found in index.js')
  }
  return indexJs.replace(BINDING_CHECK, `$1${version}$2`).replace(BINDING_MESSAGE, `$1${version}$2`)
}

function syncFile(filePath, sync, version) {
  const name = path.basename(filePath)
  const current = fs.readFileSync(filePath, 'utf8')
  const updated = sync(current, version)

  if (current === updated) {
    console.log(`${name} already at ${version}`)
  } else {
    fs.writeFileSync(filePath, updated)
    console.log(`${name} version synced to ${version}`)
  }
}

if (require.main === module) {
  const root = path.resolve(__dirname, '..')
  const version = require(path.join(root, 'package.json')).version

  try {
    syncFile(path.join(root, 'Cargo.toml'), syncCargo, version)
    syncFile(path.join(root, 'index.js'), syncIndexJs, version)
  } catch (err) {
    console.error(err.message)
    process.exit(1)
  }
}

module.exports = { syncCargo, syncIndexJs }
