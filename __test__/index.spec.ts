import test from 'ava'
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
