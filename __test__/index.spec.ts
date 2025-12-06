import test from 'ava'

import { findAffected, discoverProjects } from '../index'

test('sync function from native code', (t) => {
  t.truthy(findAffected)
  t.truthy(discoverProjects)
})
