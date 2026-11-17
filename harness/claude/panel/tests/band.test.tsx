import { expect, test } from 'claude-code/testing'

import { amendment, designStage, everyState, finished, idle, parked, verifying } from '../fixtures/factory'
import type { Scenario } from '../fixtures/factory'
import { PANE, bandProps, engine } from './support'

const BAND = { plugin: 'factory-panel', component: 'AbovePrompt', requestId: 'band' } as const
const COMMAND_ONLY = { options: { panel: 'command' } } as const

async function band($: any, on: any, scenario: Scenario | 'not-factory' | 'no-engine', props = bandProps()) {
  const env = engine(on, scenario)
  await $.session.start({ cwd: '/', surface: 'desktop', isInteractive: true })
  const ui = await $.ui.mount({ ...BAND, surface: 'desktop', props })
  return { ...env, ui }
}

async function rowText(ui: any): Promise<string> {
  const row = await ui.find({ key: 'open-panel' })
  return row?.text ?? ''
}

test('no run: the design stage with its open gate, in the warning tone', COMMAND_ONLY, async ($, on) => {
  const { ui } = await band($, on, designStage())
  const text = await rowText(ui)
  expect(text).toContain('Design')
  expect(text).toContain('The ui gate, round 1')
  expect(text).toContain('Open panel')
  await ui.unmount()
})

test('idle: names what the next unit waits for', COMMAND_ONLY, async ($, on) => {
  const { ui } = await band($, on, idle())
  const text = await rowText(ui)
  expect(text).toContain('Idle')
  expect(text).toContain('run-1')
  expect(text).toContain('it depends on U-3, which has not landed')
  await ui.unmount()
})

test('building: the units at work with their signs, three IDs and then +N', COMMAND_ONLY, async ($, on) => {
  const { ui } = await band($, on, everyState())
  const text = await rowText(ui)
  expect(text).toContain('building')
  expect(text).toContain('run-1 K-1')
  expect(text).toContain('▶ U-5')
  expect(text).toContain('◐ U-4 round 1')
  expect(text).toContain('↻ U-3 round 1')
  expect(text).toContain('+1')
  expect(text).not.toContain('U-2,')
  await ui.unmount()
})

test('needs the human: an amendment waiting is named first', COMMAND_ONLY, async ($, on) => {
  const { ui } = await band($, on, amendment())
  const text = await rowText(ui)
  expect(text).toContain('Amendment')
  expect(text).toContain('A-2')
  await ui.unmount()
})

test('blocked: parked and blocked units are counted', COMMAND_ONLY, async ($, on) => {
  const { ui } = await band($, on, parked())
  const text = await rowText(ui)
  expect(text).toContain('parked')
  expect(text).toContain('2 stuck')
  await ui.unmount()
})

test('verifying: a checkpoint being verified', COMMAND_ONLY, async ($, on) => {
  const { ui } = await band($, on, verifying())
  const text = await rowText(ui)
  expect(text).toContain('checkpoint being verified')
  await ui.unmount()
})

test('a finished run says so', COMMAND_ONLY, async ($, on) => {
  const { ui } = await band($, on, finished())
  expect(await rowText(ui)).toContain('Finished')
  await ui.unmount()
})

test('read failed: a dim line that says so and still opens the panel', COMMAND_ONLY, async ($, on) => {
  const env = engine(on, everyState())
  await $.session.start({ cwd: '/', surface: 'desktop', isInteractive: true })
  env.world.failWith = 'the engine broke'
  await $.turn.complete({ text: 'done' } as never)
  const ui = await $.ui.mount({ ...BAND, surface: 'desktop', props: bandProps() })
  const text = await rowText(ui)
  expect(text).toContain('Panel could not read the factory')
  expect(text).toContain('Open panel')
  await ui.press({ key: 'open-panel' })
  expect(env.world.panes.some(p => p.id === 'factory-panel')).toBe(true)
  await ui.unmount()
})

test('a press anywhere on the row opens the pane', COMMAND_ONLY, async ($, on) => {
  const { ui, world } = await band($, on, everyState())
  expect(world.panes.length).toBe(0)
  await ui.press({ key: 'open-panel' })
  expect(world.panes.map(p => p.id)).toEqual(['factory-panel'])
  await ui.unmount()
})

test('nothing is drawn while the pane is shown', COMMAND_ONLY, async ($, on) => {
  const { world, ui } = await band($, on, everyState())
  await ui.press({ key: 'open-panel' })
  expect(world.panes.length).toBe(1)
  const again = await $.ui.mount({ ...BAND, requestId: 'band-2', surface: 'desktop', props: bandProps() })
  expect(await again.find({ key: 'open-panel' })).toBe(undefined)
  expect((await again.find({ type: 'Text' }))?.text).toBe('engine')
  await again.unmount()
  await ui.unmount()
})

test('nothing is drawn while a survey holds the band', COMMAND_ONLY, async ($, on) => {
  const { ui } = await band($, on, everyState(), bandProps({ hasSurvey: true }))
  expect(await ui.find({ key: 'open-panel' })).toBe(undefined)
  await ui.unmount()
})

test('nothing is drawn with the option panel off', { options: { panel: 'off' } }, async ($, on) => {
  const { ui } = await band($, on, everyState())
  expect(await ui.find({ key: 'open-panel' })).toBe(undefined)
  await ui.unmount()
})

test('nothing is drawn outside a factory or with the engine missing', COMMAND_ONLY, async ($, on) => {
  const env = engine(on, 'not-factory')
  for (const scenario of ['not-factory', 'no-engine'] as const) {
    env.world.scenario = scenario
    await $.session.start({ cwd: '/', surface: 'desktop', isInteractive: true })
    const ui = await $.ui.mount({ ...BAND, surface: 'desktop', props: bandProps() })
    expect(await ui.find({ key: 'open-panel' })).toBe(undefined)
    await ui.unmount()
  }
})

test('nothing is drawn on the terminal surface, the pane included', COMMAND_ONLY, async ($, on) => {
  engine(on, everyState())
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  const ui = await $.ui.mount({ ...BAND, surface: 'terminal', props: bandProps() })
  expect(await ui.find({ key: 'open-panel' })).toBe(undefined)
  expect((await ui.find({ type: 'Text' }))?.text).toBe('engine')
  await ui.unmount()
  const pane = await $.ui.mount({
    ...PANE,
    surface: 'terminal',
    props: { title: 'x', isFocused: false, bodyColumns: 80, placement: 'dock', scroll: { offset: 0, bodyRows: 20 }, view: {} },
  })
  expect(await pane.find({ key: 'refresh' })).toBe(undefined)
  await pane.unmount()
})
