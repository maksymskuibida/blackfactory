import { expect, test } from 'claude-code/testing'

import { NOT_A_FACTORY_ANSWER, answer, everyState } from '../fixtures/factory'
import { createReader, ReadRefused } from '../src/reader'
import { isRead, viewArgs } from '../src/reads'
import { createRefresh, EMPTY_DATA } from '../src/refresh'
import type { FactoryPanelData } from '../types'

test('the reader refuses any argument list that is not in reads.ts', async () => {
  const seen: string[][] = []
  const reader = createReader(async argv => {
    seen.push(argv)
    return { exitCode: 0, stdout: '{}', stderr: '' }
  }, 'bf')
  const refused = [
    ['next'],
    ['next', '--json'],
    ['status'],
    ['status', '--json', '--extra'],
    ['record', 'put', 'unit'],
    ['record', 'list'],
    ['gate', 'approve', 'plan-r1'],
    ['view', 'nothing', '--json'],
    ['view', 'gate', 'a b', '--json'],
    ['view', 'gate', '$(id)', '--json'],
    ['view', 'gate', 'x', 'y', '--json'],
    ['render', 'index', '--json'],
    [],
  ]
  for (const args of refused) {
    expect(isRead(args)).toBe(false)
    let thrown: unknown = null
    try {
      await reader.read(args)
    } catch (err) {
      thrown = err
    }
    expect(thrown instanceof ReadRefused).toBe(true)
  }
  expect(seen.length).toBe(0)
  for (const args of [
    ['status', '--json'],
    ['progress', '--json'],
    ['record', 'list', '--json'],
    viewArgs('index'),
    viewArgs('checkpoint', 'K-2'),
  ]) {
    expect(isRead(args)).toBe(true)
    const r = await reader.read(args)
    expect(r.ok).toBe(true)
  }
  expect(seen.map(a => a.join(' '))).toEqual([
    'bf status --json',
    'bf progress --json',
    'bf record list --json',
    'bf view index --json',
    'bf view checkpoint K-2 --json',
  ])
})

test('the reader says which failure it is', async () => {
  const none = await createReader(async () => NOT_A_FACTORY_ANSWER, 'bf').read(['status', '--json'])
  expect(none.ok).toBe(false)
  if (!none.ok) expect(none.reason).toBe('not-factory')

  const missing = await createReader(async () => {
    throw new Error('spawn bf ENOENT')
  }, 'bf').read(['status', '--json'])
  if (!missing.ok) {
    expect(missing.reason).toBe('no-engine')
    expect(missing.message).toBe('spawn bf ENOENT')
  } else expect(missing.ok).toBe(false)

  const refusal = await createReader(
    async () => ({ exitCode: 2, stdout: JSON.stringify({ refused: 'read the log', because: 'the chain is broken' }), stderr: '' }),
    'bf',
  ).read(['status', '--json'])
  if (!refusal.ok) {
    expect(refusal.reason).toBe('failed')
    expect(refusal.message).toBe('the chain is broken')
  } else expect(refusal.ok).toBe(false)

  const odd = await createReader(async () => ({ exitCode: 0, stdout: 'not json', stderr: '' }), 'bf').read([
    'status',
    '--json',
  ])
  expect(odd.ok).toBe(false)

  const badName = await createReader(async () => ({ exitCode: 0, stdout: '{}', stderr: '' }), 'bf; rm').read([
    'status',
    '--json',
  ])
  expect(badName.ok).toBe(false)
})

function harness(scenario = everyState()) {
  let data: FactoryPanelData = { ...EMPTY_DATA }
  let clock = 1000
  const calls: string[] = []
  let fail = false
  const refresh = createRefresh({
    run: async argv => {
      calls.push(argv.slice(1).join(' '))
      if (fail) return { exitCode: 1, stdout: '', stderr: 'the engine broke' }
      return answer(scenario, argv)
    },
    now: async () => clock,
    bf: () => 'bf',
    data: {
      get: async () => data,
      update: async fn => {
        data = fn(data)
      },
    },
  })
  return {
    refresh,
    calls,
    scenario,
    get data() {
      return data
    },
    tick(ms: number) {
      clock += ms
    },
    failing(v: boolean) {
      fail = v
    },
  }
}

test('a full read runs the four reads and builds the model', async () => {
  const h = harness()
  expect(await h.refresh.full()).toBe('read')
  expect(h.calls).toEqual(['status --json', 'progress --json', 'record list --json', 'view index --json'])
  expect(h.data.phase).toBe('ready')
  expect(h.data.folder).toBe('factory')
  expect(h.data.model?.project).toBe('tinylist')
  expect(h.data.readAt).toBe(1000)
})

test('a poll reads the rest only when the event count changed', async () => {
  const h = harness()
  await h.refresh.full()
  h.calls.length = 0
  h.tick(5000)
  expect(await h.refresh.poll()).toBe('unchanged')
  expect(h.calls).toEqual(['status --json'])
  expect(h.data.readAt).toBe(6000)
  h.scenario.status = { ...h.scenario.status, events: 121 }
  h.calls.length = 0
  expect(await h.refresh.poll()).toBe('read')
  expect(h.calls).toEqual(['status --json', 'progress --json', 'record list --json', 'view index --json'])
  expect(h.data.events).toBe(121)
})

test('a failed read keeps the last good model and carries the engine message', async () => {
  const h = harness()
  await h.refresh.full()
  h.failing(true)
  expect(await h.refresh.full()).toBe('failed')
  expect(h.data.failed).toBe(true)
  expect(h.data.error).toBe('the engine broke')
  expect(h.data.model?.project).toBe('tinylist')
  h.failing(false)
  expect(await h.refresh.full()).toBe('read')
  expect(h.data.failed).toBe(false)
  expect(h.data.error).toBe('')
})

test('a folder that is no factory and a missing engine are told apart', async () => {
  let data: FactoryPanelData = { ...EMPTY_DATA }
  const ports = (run: (argv: string[]) => Promise<{ exitCode: number; stdout: string; stderr: string }>) => ({
    run,
    now: async () => 5,
    bf: () => 'bf',
    data: {
      get: async () => data,
      update: async (fn: (d: FactoryPanelData) => FactoryPanelData) => {
        data = fn(data)
      },
    },
  })
  await createRefresh(ports(async () => NOT_A_FACTORY_ANSWER)).full()
  expect(data.folder).toBe('not-factory')
  expect(data.model).toBe(null)
  data = { ...EMPTY_DATA }
  await createRefresh(
    ports(async () => {
      throw new Error('not found')
    }),
  ).full()
  expect(data.folder).toBe('no-engine')
  expect(data.engine).toBe('bf')
  expect(data.error).toBe('not found')
})

test('a read that rejects after a good read keeps the last good view', async () => {
  let data: FactoryPanelData = { ...EMPTY_DATA }
  let reject = false
  const scenario = everyState()
  const refresh = createRefresh({
    run: async argv => {
      if (reject) throw new Error('timed out after 20000 ms')
      return answer(scenario, argv)
    },
    now: async () => 9,
    bf: () => 'bf',
    data: {
      get: async () => data,
      update: async (fn: (d: FactoryPanelData) => FactoryPanelData) => {
        data = fn(data)
      },
    },
  })
  expect(await refresh.full()).toBe('read')
  reject = true
  expect(await refresh.full()).toBe('failed')
  expect(data.folder).toBe('factory')
  expect(data.failed).toBe(true)
  expect(data.error).toBe('timed out after 20000 ms')
  expect(data.model?.project).toBe('tinylist')

  // Even a start failure after a good read leaves the last good view in place.
  reject = false
  await refresh.full()
  const gone = createRefresh({
    run: async () => {
      throw new Error('spawn bf ENOENT')
    },
    now: async () => 9,
    bf: () => 'bf',
    data: {
      get: async () => data,
      update: async (fn: (d: FactoryPanelData) => FactoryPanelData) => {
        data = fn(data)
      },
    },
  })
  await gone.full()
  expect(data.folder).toBe('factory')
  expect(data.model?.project).toBe('tinylist')
})

test('a read that times out is a failed read, not a missing engine', async () => {
  const out = await createReader(async () => {
    throw new Error('command timed out')
  }, 'bf').read(['status', '--json'])
  expect(out.ok).toBe(false)
  if (!out.ok) expect(out.reason).toBe('failed')
})
