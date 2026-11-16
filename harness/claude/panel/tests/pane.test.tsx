import { expect, test } from 'claude-code/testing'

import {
  amendment,
  designStage,
  everyState,
  finished,
  large,
  parked,
} from '../fixtures/factory'
import type { Scenario } from '../fixtures/factory'
import { buildModel } from '../src/model'
import { PANE, engine, paneProps } from './support'

const WIDTHS = [40, 64, 120] as const

async function started($: any, on: any, scenario: Scenario | 'not-factory' | 'no-engine') {
  const env = engine(on, scenario)
  await $.session.start({ cwd: '/', surface: 'desktop', isInteractive: true })
  return env
}

async function texts(ui: any): Promise<string> {
  const all = await ui.findAll({ type: 'Text' })
  return all.map((x: { text?: string }) => x.text ?? '').join('\n')
}

test('first read in progress: placeholder rows and no model yet', async ($, on) => {
  engine(on, designStage())
  for (const columns of WIDTHS) {
    const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(columns) })
    expect(await ui.find({ type: 'Text', text: '▒' })).toBeDefined()
    expect(await ui.find({ key: 'refresh' })).toBeDefined()
    await ui.unmount()
  }
})

test('a folder that is not a factory says so in one line', async ($, on) => {
  await started($, on, 'not-factory')
  for (const columns of WIDTHS) {
    const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(columns) })
    const all = await ui.findAll({ type: 'Text' })
    expect(all.length).toBe(1)
    expect(await ui.find({ key: 'refresh' })).toBe(undefined)
    await ui.unmount()
  }
})

test('an engine that cannot be run is named with its option', { options: { bf: 'factory-bf' } }, async ($, on) => {
  await started($, on, 'no-engine')
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  const line = await ui.find({ type: 'Text', text: /factory-bf/ })
  expect(line?.text).toContain('factory-bf')
  expect(line?.text).toContain('option')
  expect(line?.text).not.toContain('ENOENT')
  await ui.unmount()
})

test('a design stage shows the open gate in the Now card at every width', async ($, on) => {
  await started($, on, designStage())
  for (const columns of WIDTHS) {
    const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(columns) })
    const all = await texts(ui)
    expect(all).toContain('tinylist')
    expect(all).toContain('Design stage')
    expect(all).toContain('The ui gate, round 1')
    expect(all).toContain('no unit has started')
    expect(await ui.find({ key: 'refresh' })).toBeDefined()
    await ui.unmount()
  }
})

test('a run going shows the progress line as printed, the checkpoint and the six steps', async ($, on) => {
  const s = everyState()
  await started($, on, s)
  for (const columns of WIDTHS) {
    const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(columns) })
    const all = await texts(ui)
    expect(all).toContain(s.progress)
    expect(all).toContain('Checkpoint K-1')
    expect(all).toContain('The first half works.')
    for (const step of ['idea', 'requirements', 'ui', 'architecture', 'plan', 'build']) {
      expect(await ui.find({ key: `tab:${step}` })).toBeDefined()
    }
    expect(all).toContain('approved')
    await ui.unmount()
  }
})

test('a parked or blocked unit puts the Now card in the warning tone with why', async ($, on) => {
  await started($, on, parked())
  for (const columns of WIDTHS) {
    const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(columns) })
    const all = await texts(ui)
    expect(all).toContain('Parked')
    expect(all).toContain('the builder asked for a decision')
    expect(all).toContain('Waits on you')
    await ui.unmount()
  }
})

test('an open amendment shows a banner with its ID and the IDs it names', async ($, on) => {
  await started($, on, amendment())
  for (const columns of WIDTHS) {
    const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(columns) })
    const all = await texts(ui)
    expect(all).toContain('Amendment A-2 names R-2, S-1')
    await ui.unmount()
  }
})

test('a finished run says every checkpoint is closed', async ($, on) => {
  await started($, on, finished())
  for (const columns of WIDTHS) {
    const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(columns) })
    const all = await texts(ui)
    expect(all).toContain('The run is finished')
    expect(all).toContain('2 of 2 checkpoints closed')
    await ui.unmount()
  }
})

test('a failed read shows a red line with the engine message and keeps the last view', async ($, on) => {
  const { world } = await started($, on, everyState())
  const probe = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  await probe.unmount()
  // The engine breaks after the first good read: a refresh fails, the model stays.
  world.failWith = 'the engine broke'
  await $.turn.complete({ text: 'done' } as never)
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  const all = await texts(ui)
  expect(all).toContain('The last read failed: the engine broke')
  expect(all).toContain('tinylist')
  await ui.unmount()
})

test('the pane is drawn on the desktop surface and nothing else is asked of other surfaces', async ($, on) => {
  await started($, on, everyState())
  for (const surface of ['terminal', 'vscode', 'mobile'] as const) {
    const ui = await $.ui.mount({ ...PANE, surface, props: paneProps(64) })
    expect(await ui.find({ key: 'refresh' })).toBe(undefined)
    expect((await ui.find({ type: 'Text' }))?.text).toBe('engine')
    await ui.unmount()
  }
})

test('a press on a step opens its tab and Refresh reads again', async ($, on) => {
  const { world } = await started($, on, everyState())
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  const before = world.calls.length
  await ui.press({ key: 'refresh' })
  expect(world.calls.length).toBeGreaterThan(before)
  await ui.press({ key: 'tab:requirements' })
  const all = await texts(ui)
  expect(all).toContain('R-1')
  await ui.unmount()
})

test('a factory of 60 units, 30 components and 500 records builds and draws in under 300 ms', async ($, on) => {
  const s = large()
  expect(s.records.length).toBeGreaterThanOrEqual(500)
  const t0 = Date.now()
  const m = buildModel({ status: s.status, progress: { line: s.progress }, records: s.records, index: s.index })
  const built = Date.now() - t0
  expect(m.units.length).toBe(60)
  expect(Object.keys(m.records).length).toBeGreaterThanOrEqual(500)
  expect(built).toBeLessThan(300)
  await started($, on, s)
  // The first drawing of a session warms the engine's own paths; the drawing that counts is the
  // median of the next three, as the person sees it while the pane stays open.
  const warm = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(120) })
  await warm.unmount()
  const spent: number[] = []
  for (let i = 0; i < 3; i++) {
    const t1 = Date.now()
    const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(120) })
    spent.push(Date.now() - t1)
    expect(await ui.find({ key: 'refresh' })).toBeDefined()
    await ui.unmount()
  }
  spent.sort((a, b) => a - b)
  expect(built + spent[1]!).toBeLessThan(300)
})
