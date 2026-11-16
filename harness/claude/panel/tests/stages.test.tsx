import { expect, test as plain } from 'claude-code/testing'

// The runner cuts a test at 5000 ms. These tests read, press and mount several times, so under
// machine load they get 30 s; the 300 ms bar of the timing tests below is measured, not this limit.
const test = (name: string, fn: (...args: any[]) => unknown) => plain(name, { timeoutMs: 30000 }, fn as never)

import { huge, large, planView, runView, schemes, everyState } from '../fixtures/factory'
import type { Scenario } from '../fixtures/factory'
import { BOX_LIMIT, PICTURE_LIMIT, componentPicture, componentScheme, planMap, planPicture } from '../src/layout'
import { buildModel } from '../src/model'
import { parsePlanView, parseRunView } from '../src/views'
import { PANE, engine, paneProps } from './support'

declare const console: { log: (...parts: unknown[]) => void }

const WIDTHS = [40, 64, 120] as const

function calls(world: { calls: string[] }, what: string): number {
  return world.calls.filter(c => c === what).length
}

async function started($: any, on: any, scenario: Scenario) {
  const env = engine(on, scenario)
  await $.session.start({ cwd: '/', surface: 'desktop', isInteractive: true })
  return env
}

async function texts(ui: any): Promise<string> {
  const all = await ui.findAll({ type: 'Text' })
  return all.map((x: { text?: string }) => x.text ?? '').join('\n')
}

async function keys(ui: any, type = 'Button'): Promise<string[]> {
  const all = await ui.findAll({ type })
  return all.map((x: { key?: string }) => x.key ?? '')
}

// The keys of the buttons that open one record, wherever they stand on the screen.
async function opens(ui: any, id: string): Promise<string[]> {
  return (await keys(ui)).filter(k => k === `open:${id}` || k.startsWith(`open:${id}@`))
}

async function go(ui: any, key: string) {
  await ui.press({ key })
}

// Presses the first button on the screen that opens a record, wherever it stands.
async function to(ui: any, id: string) {
  const found = await opens(ui, id)
  expect(found.length).toBeGreaterThan(0)
  await ui.press({ key: found[0]! })
}

async function buttonTexts(ui: any): Promise<string> {
  const all = await ui.findAll({ type: 'Button' })
  return all.map((x: { text?: string }) => x.text ?? '').join('\n')
}

function modelOf(s: Scenario) {
  return buildModel({ status: s.status, progress: { line: s.progress }, records: s.records, index: s.index })
}

for (const columns of WIDTHS) {
  for (const tab of ['idea', 'requirements', 'ui', 'architecture', 'plan']) {
  test(`the ${tab} tab is drawn at ${columns} columns with a row per record and the engine section titles`, async ($, on) => {
  await started($, on, schemes())
  {
    const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(columns) })
    {
      await go(ui, `tabs:${tab}`)
      const all = await texts(ui)
      expect(await ui.find({ key: 'search' })).toBeDefined()
      if (tab === 'idea') expect(all).toContain('A small tool that keeps a list')
      if (tab === 'requirements') {
        expect(all).toContain('What the tool must do')
        expect(await opens(ui, 'R-1')).toHaveLength(1)
        expect(await opens(ui, 'R-2')).toHaveLength(1)
        expect(all).toContain('critical')
        expect(all).toContain('The tool keeps a list.')
        // the trace: the units that deliver it and the checkpoint that proves it
        expect(all).toContain('Built by')
        expect(all).toContain('Proved at')
      }
      if (tab === 'ui') {
        expect(all).toContain('Screens of the tool')
        expect(all).toContain('Journeys')
      }
      if (tab === 'architecture') {
        expect(all).toContain('Parts that hold it together')
        expect(all).toContain('Decisions')
        expect(all).toContain('Targets')
        expect(all).toContain('Threats')
      }
      if (tab === 'plan') {
        expect(all).toContain('Units in waves')
        expect(all).toContain('Checkpoints to close')
        expect(all).toContain('Checks')
      }
    }
    await ui.unmount()
  }
  })
  }
}

test('rows carry the unit state, the check result and whether a checkpoint is closed', async ($, on) => {
  await started($, on, schemes())
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(120) })
  await go(ui, 'tabs:plan')
  const all = await texts(ui)
  expect(all).toContain('landed')
  expect(all).toContain('passed')
  expect(all).toContain('failed')
  expect(all).toContain('not run')
  expect(all).toContain('open')
  await ui.unmount()
})

test('a kind with no records says so in one quiet line and the search finds by ID or by a word', async ($, on) => {
  await started($, on, schemes())
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  await go(ui, 'tabs:architecture')
  expect(await texts(ui)).toContain('No records of this kind yet')
  await go(ui, 'tabs:requirements')
  await ui.input({ key: 'search', text: 'restart' })
  let all = await texts(ui)
  expect(all).toContain('The list survives a restart.')
  expect(all).not.toContain('The tool keeps a list.')
  await ui.input({ key: 'search', text: 'R-1' })
  all = await texts(ui)
  expect(all).toContain('The tool keeps a list.')
  expect(all).not.toContain('The list survives a restart.')
  await ui.input({ key: 'search', text: 'nothing like this' })
  expect(await texts(ui)).toContain('Nothing matches this search')
  await go(ui, 'search-clear')
  expect(await texts(ui)).toContain('The list survives a restart.')
  await ui.unmount()
})

test('a long section folds after twenty rows with Show N more', async ($, on) => {
  await started($, on, large())
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  await go(ui, 'tabs:requirements')
  expect((await keys(ui)).filter(k => /^open:R-\d+$/.test(k))).toHaveLength(20)
  const fold = (await keys(ui)).find(k => k.startsWith('fold:sec:requirements'))
  expect(fold).toBeDefined()
  expect(await buttonTexts(ui)).toMatch(/Show \d+ more/)
  await go(ui, fold!)
  expect((await keys(ui)).filter(k => /^open:R-\d+$/.test(k)).length).toBeGreaterThan(20)
  await ui.unmount()
})

test('records named by an open amendment carry its ID', async ($, on) => {
  await started($, on, schemes())
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(120) })
  await go(ui, 'tabs:architecture')
  expect(await texts(ui)).toContain('A-7')
  await go(ui, 'open:C-3')
  expect(await texts(ui)).toContain('A-7')
  await go(ui, 'back')
  await ui.unmount()
})

test('a requirement opens with its fields, links both ways and every ID a button, and back returns along the way', async ($, on) => {
  await started($, on, schemes())
  for (const columns of WIDTHS) {
    const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(columns) })
    await go(ui, 'tabs:requirements')
    await go(ui, 'open:R-1')
    const all = await texts(ui)
    expect(all).toContain('The tool keeps a list.')
    expect(all).toContain('Acceptance')
    expect(all).toContain('It lists.')
    expect(all).toContain('Links to')
    expect(all).toContain('Linked from')
    expect(await opens(ui, 'D-1')).not.toHaveLength(0)
    // what links to it: a screen, a journey, a component
    expect(await opens(ui, 'S-1')).not.toHaveLength(0)
    expect(await opens(ui, 'J-1')).not.toHaveLength(0)
    // the trace names the unit and the checkpoint
    expect(await opens(ui, 'U-1')).not.toHaveLength(0)
    expect(await opens(ui, 'K-1')).not.toHaveLength(0)
    await to(ui, 'S-1')
    expect(await texts(ui)).toContain('The list screen.')
    expect(await texts(ui)).toContain('Interface: page.')
    await to(ui, 'J-1')
    expect(await texts(ui)).toContain('The person adds an item.')
    await go(ui, 'back')
    expect(await texts(ui)).toContain('The list screen.')
    await go(ui, 'back')
    expect(await texts(ui)).toContain('Acceptance')
    await go(ui, 'back')
    expect(await ui.find({ key: 'search' })).toBeDefined()
    await ui.unmount()
  }
})

test('an ID in a text is a button, an ID that is no record is dim and not pressable, the crumb goes back', async ($, on) => {
  await started($, on, schemes())
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  await go(ui, 'tabs:architecture')
  await go(ui, 'open:C-2')
  const all = await texts(ui)
  expect(all).toContain('Z-99')
  expect(await opens(ui, 'Z-99')).toHaveLength(0)
  const inText = await opens(ui, 'U-1')
  expect(inText.length).toBeGreaterThan(0)
  await go(ui, inText[0]!)
  expect(await texts(ui)).toContain('Unit 1 builds a part.')
  expect(await ui.find({ key: 'crumb:0' })).toBeDefined()
  await go(ui, 'crumb:0')
  expect(await texts(ui)).toContain('The view shows the list.')
  await ui.unmount()
})

test('a unit shows its files, checks, checkpoint, state and what it waits for; a check its command and receipt', async ($, on) => {
  await started($, on, schemes())
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  await go(ui, 'tabs:plan')
  await go(ui, 'open:U-7')
  let all = await texts(ui)
  expect(all).toContain('planned')
  expect(all).toContain('which has not landed')
  expect(await opens(ui, 'U-5')).not.toHaveLength(0)
  expect(all).toContain('Files')
  expect(all).toContain('Delivers')
  expect(await opens(ui, 'X-7')).not.toHaveLength(0)
  expect(await opens(ui, 'K-2')).not.toHaveLength(0)
  await go(ui, 'back')
  await go(ui, 'tabs:plan')
  await go(ui, 'open:X-2')
  all = await texts(ui)
  expect(all).toContain('cargo test')
  expect(all).toContain('failed')
  await to(ui, 'U-2')
  await to(ui, 'K-1')
  all = await texts(ui)
  expect(all).toContain('The first half works.')
  expect(all).toContain('Waits for')
  await ui.unmount()
})

test('the reviewer measures sentences are set apart', async ($, on) => {
  const s = everyState()
  const u = s.records.find(r => r.id === 'U-1')!
  u.text = 'Builds the part. The reviewer measures the picture size against 131072 characters. Then it lands.'
  await started($, on, s)
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  await go(ui, 'tabs:plan')
  await go(ui, 'open:U-1')
  const all = await texts(ui)
  expect(all).toContain('Reviewer measures')
  expect(all).toContain('The reviewer measures the picture size against 131072 characters.')
  await ui.unmount()
})

test('Open on the full page is drawn only for a recorded https page, else a quiet line says so', async ($, on) => {
  await started($, on, schemes())
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  // a unit goes to the plan page
  await go(ui, 'tabs:plan')
  await go(ui, 'open:U-1')
  let links = await ui.findAll({ type: 'Link' })
  expect(links.map((l: any) => l.props.href)).toEqual(['https://example.invalid/plan'])
  // a requirement has no gate page recorded
  await go(ui, 'tabs:requirements')
  await go(ui, 'open:R-1')
  expect(await ui.findAll({ type: 'Link' })).toHaveLength(0)
  expect(await texts(ui)).toContain('This page is not published yet')
  // a design part of the architecture takes the latest round
  await go(ui, 'tabs:architecture')
  await go(ui, 'open:C-1')
  links = await ui.findAll({ type: 'Link' })
  expect(links.map((l: any) => l.props.href)).toEqual(['https://example.invalid/gate-architecture-r2'])
  // a checkpoint takes checkpoint-<K>
  await go(ui, 'tabs:plan')
  await go(ui, 'open:K-1')
  links = await ui.findAll({ type: 'Link' })
  expect(links.map((l: any) => l.props.href)).toEqual(['https://example.invalid/checkpoint-K-1'])
  // the latest round of the ui gate is an http page: no link
  await go(ui, 'tabs:ui')
  await go(ui, 'open:S-1')
  expect(await ui.findAll({ type: 'Link' })).toHaveLength(0)
  await ui.unmount()
})

test('the component scheme has its layers, its lines, a warning row and a text form that opens records', async ($, on) => {
  const s = schemes()
  const m = modelOf(s)
  const scheme = componentScheme(m)
  expect(scheme.layers.map(l => l.ids)).toEqual([['C-4'], ['C-3'], ['C-2'], ['C-1']])
  expect(scheme.edges).toHaveLength(4)
  expect(scheme.missing).toEqual([['C-4', 'C-9']])
  const pic = componentPicture(m, scheme)!
  expect(pic.source.match(/<line /g)).toHaveLength(4)
  expect(pic.source).toContain('prefers-color-scheme: dark')
  expect(pic.source).toContain('.s-landed')
  await started($, on, s)
  for (const columns of WIDTHS) {
    const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(columns) })
    await go(ui, 'tabs:architecture')
    expect(await ui.find({ type: 'Svg' })).toBeDefined()
    expect(await texts(ui)).toContain('Warning: C-4 links to C-9, which is not a record')
    expect(await texts(ui)).toContain('Layer 1')
    const inForm = (await keys(ui)).filter(k => k.startsWith('open:C-2@sc'))
    expect(inForm).toHaveLength(1)
    await go(ui, inForm[0]!)
    expect(await texts(ui)).toContain('The view shows the list.')
    await ui.unmount()
  }
})

test('components that link to no other draw boxes without lines and say so', async ($, on) => {
  await started($, on, everyState())
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  await go(ui, 'tabs:architecture')
  expect(await ui.find({ type: 'Svg' })).toBeDefined()
  expect(await texts(ui)).toContain('No component links to another')
  await ui.unmount()
})

test('the plan map has a line per edge, units filled by state, checkpoint blocks and a text form', async ($, on) => {
  const s = schemes()
  const m = modelOf(s)
  expect(Object.keys(parseRunView(runView(s))).length).toBeGreaterThan(0)
  const map = planMap(parsePlanView(planView(s)))
  expect(map.waves).toHaveLength(2)
  expect(map.edges.length).toBeGreaterThan(0)
  const pic = planPicture(m, map, { closed: 'closed', open: 'open', stop: 'stop' })!
  expect(pic.source.match(/<line /g)).toHaveLength(map.edges.length)
  expect(pic.source).toContain('class="s-landed"')
  expect(pic.source).toContain('class="s-parked"'.replace('parked', 'planned'))
  expect(pic.source).toContain('K-1')
  await started($, on, s)
  for (const columns of WIDTHS) {
    const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(columns) })
    await go(ui, 'tabs:plan')
    expect(await ui.find({ type: 'Svg' })).toBeDefined()
    expect(await texts(ui)).toContain('Wave 1')
    const inForm = (await keys(ui)).filter(k => k.startsWith('open:U-7@pw'))
    expect(inForm).toHaveLength(1)
    await go(ui, inForm[0]!)
    expect(await texts(ui)).toContain('which has not landed')
    await ui.unmount()
  }
})

test('a parked unit shows in the plan map and what waits for it is on its page', async ($, on) => {
  const s = everyState()
  ;(s.status.units as any[]).find(u => u.id === 'U-3').state = 'parked'
  ;(s.status.units as any[]).find(u => u.id === 'U-3').waiting_for = 'the builder asked for a decision'
  await started($, on, s)
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  await go(ui, 'tabs:plan')
  const svg = await ui.find({ type: 'Svg' })
  expect(svg?.props.source).toContain('s-parked')
  await go(ui, (await keys(ui)).find(k => k.startsWith('open:U-3@pw'))!)
  expect(await texts(ui)).toContain('the builder asked for a decision')
  await ui.unmount()
})

test('a failed plan read says so, never "no plan yet" or "not run", and refresh and a returning pane read again', async ($, on) => {
  const { world } = await started($, on, schemes())
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  world.failWith = 'the engine broke'
  await go(ui, 'tabs:plan')
  let all = await texts(ui)
  expect(all).toContain('The engine could not be read for this page')
  expect(all).not.toContain('No plan yet')
  expect(await ui.find({ type: 'Svg' })).toBe(undefined)
  expect(all).not.toContain('not run')
  world.failWith = null
  const before = calls(world, 'view plan --json')
  await $.turn.complete({ text: 'done' } as never)
  expect(calls(world, 'view plan --json')).toBeGreaterThan(before)
  expect(await ui.find({ type: 'Svg' })).toBeDefined()
  expect(await texts(ui)).not.toContain('The engine could not be read for this page')
  await to(ui, 'U-1')
  const runs = calls(world, 'view run --json')
  await $.turn.complete({ text: 'done' } as never)
  expect(calls(world, 'view run --json')).toBeGreaterThan(runs)
  await ui.unmount()
})

test('a checkpoint with an empty human step does not stop', async () => {
  const s = schemes()
  s.records.find(r => r.id === 'K-1')!.human_step = ''
  const m = modelOf(s)
  const map = planMap(parsePlanView(planView(s)))
  const pic = planPicture(m, map, { closed: 'closed', open: 'open', stop: 'stop' })!
  expect(pic.source).not.toContain('open · stop')
  s.records.find(r => r.id === 'K-1')!.human_step = 'Look at it.'
  const stops = planPicture(modelOf(s), map, { closed: 'closed', open: 'open', stop: 'stop' })!
  expect(stops.source).toContain('open · stop')
})

test('no plan yet says so in one line', async ($, on) => {
  const s = everyState()
  s.records = s.records.filter(r => r.kind !== 'unit' && r.kind !== 'check' && r.kind !== 'checkpoint')
  await started($, on, s)
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  await go(ui, 'tabs:plan')
  expect(await ui.find({ type: 'Svg' })).toBe(undefined)
  expect(await texts(ui)).toContain('No plan yet')
  await ui.unmount()
})

test('a plan of 200 units is not drawn: the text form and the link stay', async ($, on) => {
  const s = huge()
  await started($, on, s)
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(64) })
  await go(ui, 'tabs:plan')
  expect(await ui.find({ type: 'Svg' })).toBe(undefined)
  const all = await texts(ui)
  expect(all).toContain('too large to draw')
  expect(all).toContain('Wave 40')
  expect((await keys(ui)).filter(k => k.includes('@pw40.')).length).toBe(5)
  const links = await ui.findAll({ type: 'Link' })
  expect(links.map((l: any) => l.props.href)).toContain('https://example.invalid/plan')
  await ui.unmount()
  expect(planPicture(modelOf(s), planMap(parsePlanView(planView(s))), { closed: '', open: '', stop: '' })).toBe(null)
})

test('a factory of 60 units, 30 components and 500 records draws each screen and both pictures in under 300 ms', async ($, on) => {
  const s = large()
  const m = modelOf(s)
  const views = parsePlanView(planView(s))
  const t0 = Date.now()
  const cp = componentPicture(m, componentScheme(m))
  const pp = planPicture(m, planMap(views), { closed: 'closed', open: 'open', stop: 'stop' })
  const drawn = Date.now() - t0
  expect(cp).not.toBe(null)
  expect(pp).not.toBe(null)
  console.log(`component picture: ${cp!.size} characters; plan map picture: ${pp!.size} characters; limit ${PICTURE_LIMIT}; ${m.units.length} units (box limit ${BOX_LIMIT}); drawn in ${drawn} ms`)
  expect(cp!.size).toBeLessThan(PICTURE_LIMIT)
  expect(pp!.size).toBeLessThan(PICTURE_LIMIT)
  expect(drawn).toBeLessThan(300)
})

// Each screen of the large factory: the first drawing warms the engine's own paths, the drawing that
// counts is the median of the five after it, as the person sees it while the pane stays open.
for (const screen of ['requirements', 'ui', 'architecture', 'plan', 'record', 'unit']) {
  test(`the ${screen} screen of 60 units, 30 components and 500 records draws in under 300 ms`, async ($, on) => {
    await started($, on, large())
    const setup = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(120) })
    await go(setup, `tabs:${screen === 'unit' ? 'plan' : screen === 'record' ? 'architecture' : screen}`)
    if (screen === 'record') await go(setup, 'open:C-1')
    if (screen === 'unit') await go(setup, 'open:U-1')
    await setup.unmount()
    const spent: number[] = []
    for (let i = 0; i < 6; i++) {
      const t1 = Date.now()
      const ui = await $.ui.mount({ ...PANE, surface: 'desktop', props: paneProps(120) })
      spent.push(Date.now() - t1)
      await ui.unmount()
    }
    spent.shift()
    spent.sort((a, b) => a - b)
    console.log(`${screen}: ${spent[2]} ms`)
    expect(spent[2]!).toBeLessThan(300)
  })
}
