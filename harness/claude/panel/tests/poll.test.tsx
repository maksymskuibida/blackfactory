import { expect, test } from 'claude-code/testing'

import { everyState } from '../fixtures/factory'
import { BAND_FOR_TESTS, PANE, bandProps, engine, paneProps, statusCalls } from './support'

const COMMAND_ONLY = { options: { panel: 'command' } } as const

function restCalls(calls: string[]): number {
  return calls.filter(c => c !== 'status --json').length
}

async function begin($: any, on: any) {
  const env = engine(on, everyState())
  await $.session.start({ cwd: '/', surface: 'desktop', isInteractive: true })
  return env
}

test('nothing is polled while the pane is not placed and the status line is not on screen', COMMAND_ONLY, async ($, on) => {
  const { world, clock } = await begin($, on)
  expect(statusCalls(world)).toBe(1)
  await clock.advance(60_000)
  expect(statusCalls(world)).toBe(1)
})

test('with the status line on screen the engine runs one status read per interval', COMMAND_ONLY, async ($, on) => {
  const { world, clock } = await begin($, on)
  const ui = await $.ui.mount({ ...BAND_FOR_TESTS, surface: 'desktop', props: bandProps() })
  const rest = restCalls(world.calls)
  await clock.advance(5_000)
  expect(statusCalls(world)).toBe(2)
  await clock.advance(5_000)
  expect(statusCalls(world)).toBe(3)
  await clock.advance(15_000)
  expect(statusCalls(world)).toBe(6)
  // The count never changed: nothing else was read.
  expect(restCalls(world.calls)).toBe(rest)
  await ui.unmount()
})

test('the rest is read once when the event count changed', COMMAND_ONLY, async ($, on) => {
  const { world, clock } = await begin($, on)
  const ui = await $.ui.mount({ ...BAND_FOR_TESTS, surface: 'desktop', props: bandProps() })
  const rest = restCalls(world.calls)
  if (world.scenario !== 'not-factory' && world.scenario !== 'no-engine') {
    world.scenario.status = { ...world.scenario.status, events: 999 }
  }
  await clock.advance(5_000)
  expect(restCalls(world.calls)).toBe(rest + 3)
  await clock.advance(5_000)
  expect(restCalls(world.calls)).toBe(rest + 3)
  await ui.unmount()
})

test('a changed event count reads the plan and run views again for the open tab and record', { ...COMMAND_ONLY, timeoutMs: 30000 }, async ($, on) => {
  const { world, clock } = await begin($, on)
  await $.command.run({ command: 'factory-panel', args: '' } as never)
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  await ui.press({ key: 'tabs:plan' })
  const count = (what: string) => world.calls.filter(c => c === what).length
  const plans = count('view plan --json')
  const runs = count('view run --json')
  // The count did not change: the views are not read again.
  await clock.advance(5_000)
  expect(count('view plan --json')).toBe(plans)
  if (world.scenario !== 'not-factory' && world.scenario !== 'no-engine') {
    world.scenario.status = { ...world.scenario.status, events: 999 }
  }
  await clock.advance(5_000)
  expect([plans, count('view plan --json')]).toEqual([plans, plans + 1])
  expect(count('view run --json')).toBe(runs + 1)
  await clock.advance(5_000)
  expect(count('view plan --json')).toBe(plans + 1)
  // A record open on a unit: its receipts are read again on the next change.
  const keys = (await ui.findAll({ type: 'Button' })).map((b: { key?: string }) => b.key ?? '')
  await ui.press({ key: keys.find((k: string) => k.startsWith('open:U-1@'))! })
  const afterOpen = count('view run --json')
  if (world.scenario !== 'not-factory' && world.scenario !== 'no-engine') {
    world.scenario.status = { ...world.scenario.status, events: 1000 }
  }
  await clock.advance(5_000)
  expect(count('view run --json')).toBe(afterOpen + 1)
  await ui.unmount()
})

test('a collapsed status line is not polled for', COMMAND_ONLY, async ($, on) => {
  const { world, clock } = await begin($, on)
  const ui = await $.ui.mount({ ...BAND_FOR_TESTS, surface: 'desktop', props: bandProps({ isCollapsed: true }) })
  await clock.advance(30_000)
  expect(statusCalls(world)).toBe(1)
  await ui.unmount()
})

test('a survey holding the band stops the poll too', COMMAND_ONLY, async ($, on) => {
  const { world, clock } = await begin($, on)
  const ui = await $.ui.mount({ ...BAND_FOR_TESTS, surface: 'desktop', props: bandProps({ hasSurvey: true }) })
  await clock.advance(30_000)
  expect(statusCalls(world)).toBe(1)
  await ui.unmount()
})

test('while the pane is placed the poll runs', COMMAND_ONLY, async ($, on) => {
  const { world, clock } = await begin($, on)
  const result = await $.command.run({ command: 'factory-panel', args: '' } as never)
  expect(JSON.stringify(result)).toContain('open')
  expect(world.panes.length).toBe(1)
  const before = statusCalls(world)
  await clock.advance(10_000)
  expect(statusCalls(world)).toBe(before + 2)
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  expect((await ui.find({ key: 'refresh' }))?.key).toBe('refresh')
  await ui.unmount()
})

test('a poll of 0 is off and one under 3 seconds counts as 3', { options: { panel: 'command', pollSeconds: 0 } }, async ($, on) => {
  const { world, clock } = await begin($, on)
  const ui = await $.ui.mount({ ...BAND_FOR_TESTS, surface: 'desktop', props: bandProps() })
  await clock.advance(60_000)
  expect(statusCalls(world)).toBe(1)
  await ui.unmount()
})

test('a poll of 1 second is read as 3', { options: { panel: 'command', pollSeconds: 1 } }, async ($, on) => {
  const { world, clock } = await begin($, on)
  const ui = await $.ui.mount({ ...BAND_FOR_TESTS, surface: 'desktop', props: bandProps() })
  await clock.advance(3_000)
  expect(statusCalls(world)).toBe(2)
  await ui.unmount()
})

test('the stamp moves with each read', COMMAND_ONLY, async ($, on) => {
  const { clock } = await begin($, on)
  await $.command.run({ command: 'factory-panel', args: '' } as never)
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  expect((await ui.find({ type: 'Text', text: /read/ }))?.text).toMatch(/read (just now|\d+s ago)/)
  await clock.advance(5_000)
  expect((await ui.find({ type: 'Text', text: /read/ }))?.text).toMatch(/read (just now|\d+s ago)/)
  await ui.unmount()
})
