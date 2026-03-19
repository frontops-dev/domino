#!/usr/bin/env node

const fs = require('fs')
const path = require('path')

const root = path.resolve(__dirname, '..')
const version = require(path.join(root, 'package.json')).version
const cargoPath = path.join(root, 'Cargo.toml')

const cargo = fs.readFileSync(cargoPath, 'utf8')
const versionPattern = /^(version\s*=\s*)".*"/m

if (!versionPattern.test(cargo)) {
  console.error('No version field found in Cargo.toml')
  process.exit(1)
}

const updated = cargo.replace(versionPattern, `$1"${version}"`)

if (cargo === updated) {
  console.log(`Cargo.toml already at ${version}`)
} else {
  fs.writeFileSync(cargoPath, updated)
  console.log(`Cargo.toml version synced to ${version}`)
}
