import test from 'ava'
import { execFileSync, spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { findAffected, discoverProjects } from '../index'

test('sync function from native code', (t) => {
  t.truthy(findAffected)
  t.truthy(discoverProjects)
})

test('discoverProjects exposes project links declared in target config', (t) => {
  const root = mkdtempSync(join(tmpdir(), 'domino-config-deps-'))
  t.teardown(() => rmSync(root, { recursive: true, force: true }))

  writeFileSync(join(root, 'nx.json'), '{}')
  mkdirSync(join(root, 'libs/api'), { recursive: true })
  writeFileSync(join(root, 'libs/api/project.json'), JSON.stringify({ name: 'api' }))
  mkdirSync(join(root, 'apps/web'), { recursive: true })
  writeFileSync(
    join(root, 'apps/web/project.json'),
    JSON.stringify({
      name: 'web',
      targets: { deploy: { dependsOn: [{ projects: ['api'], target: 'build' }] } },
    }),
  )

  const web = discoverProjects(root).find((p) => p.name === 'web')
  t.deepEqual(web?.configDependencies, ['api'])
})

// findAffected diffs with git; the Linux binding-test containers have none.
const hasGit = spawnSync('git', ['--version']).status === 0
const testWithGit = hasGit ? test : test.skip

testWithGit('findAffected applies the root tsconfig strategy', (t) => {
  const root = mkdtempSync(join(tmpdir(), 'domino-root-tsconfig-'))
  t.teardown(() => rmSync(root, { recursive: true, force: true }))
  const git = (...args: string[]) => execFileSync('git', args, { cwd: root, stdio: 'ignore' })

  writeFileSync(join(root, 'nx.json'), '{}')
  for (const name of ['lib-a', 'lib-b']) {
    mkdirSync(join(root, 'libs', name, 'src'), { recursive: true })
    writeFileSync(join(root, 'libs', name, 'project.json'), JSON.stringify({ name, sourceRoot: `libs/${name}/src` }))
    writeFileSync(join(root, 'libs', name, 'src/index.ts'), 'export const value = 1\n')
  }
  // lib-b imports an alias that only exists once the branch adds it.
  writeFileSync(join(root, 'libs/lib-b/src/index.ts'), "import { value } from '@acme/a'\nexport const b = value\n")
  writeFileSync(join(root, 'tsconfig.base.json'), '{ "compilerOptions": { "paths": {} } }')
  git('init', '-q', '-b', 'main')
  git('-c', 'user.email=t@example.com', '-c', 'user.name=t', 'commit', '-q', '--allow-empty', '-m', 'init')
  git('add', '.')
  git('-c', 'user.email=t@example.com', '-c', 'user.name=t', 'commit', '-q', '-m', 'base')
  git('checkout', '-q', '-b', 'feature')
  writeFileSync(
    join(root, 'tsconfig.base.json'),
    '{ "compilerOptions": { "paths": { "@acme/a": ["libs/lib-a/src/index.ts"] } } }',
  )
  git('-c', 'user.email=t@example.com', '-c', 'user.name=t', 'commit', '-q', '-am', 'alias')

  const options = { cwd: root, base: 'main', projects: discoverProjects(root), lockfileStrategy: 'none' }

  // Default (aliases): only the project importing the added alias.
  t.deepEqual(findAffected(options).affectedProjects, ['lib-b'])
  t.deepEqual(findAffected({ ...options, rootTsconfigStrategy: 'all' }).affectedProjects.sort(), ['lib-a', 'lib-b'])
  t.deepEqual(findAffected({ ...options, rootTsconfigStrategy: 'none' }).affectedProjects, [])
  t.throws(() => findAffected({ ...options, rootTsconfigStrategy: 'some' }), {
    message: /Invalid root tsconfig strategy/,
  })
})
