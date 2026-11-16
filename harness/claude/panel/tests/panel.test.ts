import { expect, test } from 'claude-code/testing'

import { NOT_A_FACTORY_ANSWER, answer, everyState } from '../fixtures/factory'
import { EMPTY_CHOICES, EMPTY_SESSION, createPanel, normalizeOptions } from '../src/panel'
import type { PanelOptions } from '../src/panel'
import { EMPTY_DATA } from '../src/refresh'
import type { FactoryPanelChoices, FactoryPanelData, FactoryPanelSession } from '../types'

// The panel's decisions, run on ports with no engine at all.
function rig(over: { options?: Partial<PanelOptions>; desktop?: boolean; run?: 'ok' | 'not-factory' | 'no-engine' } = {}) {
  const options: PanelOptions = { panel: 'auto', pollSeconds: 5, bf: 'bf', ...over.options }
  let data: FactoryPanelData = { ...EMPTY_DATA }
  let session: FactoryPanelSession = { ...EMPTY_SESSION, isDesktop: over.desktop ?? true }
  let choices: FactoryPanelChoices = { ...EMPTY_CHOICES }
  const log = { opens: 0, runs: [] as string[], placed: false, band: false, pane: false }
  const mode = over.run ?? 'ok'
  const panel = createPanel({
    run: async argv => {
      log.runs.push(argv.slice(1).join(' '))
      if (mode === 'no-engine') throw new Error('not found')
      if (mode === 'not-factory') return NOT_A_FACTORY_ANSWER
      return answer(everyState(), argv)
    },
    now: async () => 10,
    bf: () => options.bf,
    data: {
      get: async () => data,
      update: async fn => {
        data = fn(data)
      },
    },
    options: () => options,
    session: {
      get: async () => session,
      update: async fn => {
        session = fn(session)
      },
    },
    choices: {
      update: async fn => {
        choices = fn(choices)
      },
    },
    ui: {
      open: async () => {
        log.opens += 1
        log.pane = true
        return true
      },
      isPlaced: async () => log.pane,
    },
    isBandDrawn: () => log.band,
  })
  return { panel, log, get session() { return session }, get data() { return data }, get choices() { return choices } }
}

test('the options are clamped: the poll is 0 or at least 3 seconds', () => {
  expect(normalizeOptions({}).pollSeconds).toBe(5)
  expect(normalizeOptions({ pollSeconds: 0 }).pollSeconds).toBe(0)
  expect(normalizeOptions({ pollSeconds: 2 }).pollSeconds).toBe(3)
  expect(normalizeOptions({ pollSeconds: 12 }).pollSeconds).toBe(12)
  expect(normalizeOptions({ panel: 'other' }).panel).toBe('auto')
  expect(normalizeOptions({ panel: 'off' }).panel).toBe('off')
  expect(normalizeOptions({ bf: '  ' }).bf).toBe('bf')
})

test('in a desktop session in a factory the pane opens by itself after a read succeeded', async () => {
  const r = rig()
  await r.panel.attach(true)
  expect(r.log.runs[0]).toBe('status --json')
  expect(r.log.opens).toBe(1)
  expect(r.session.isShown).toBe(true)
  await r.panel.attach(true)
  expect(r.log.opens).toBe(1)
})

test('it does not open by itself with the option command or off', async () => {
  for (const panel of ['command', 'off'] as const) {
    const r = rig({ options: { panel } })
    await r.panel.attach(true)
    expect(r.log.opens).toBe(0)
  }
})

test('off reads nothing at all', async () => {
  const r = rig({ options: { panel: 'off' } })
  await r.panel.attach(true)
  await r.panel.turnComplete()
  expect(r.log.runs.length).toBe(0)
})

test('on a surface other than the desktop app nothing is read or opened and the command says so', async () => {
  const r = rig({ desktop: false })
  await r.panel.attach(false)
  await r.panel.turnComplete()
  expect(r.log.runs.length).toBe(0)
  expect(r.log.opens).toBe(0)
  expect(await r.panel.command()).toContain('desktop app')
})

test('a pane the person closed stays closed for the session', async () => {
  const r = rig()
  await r.panel.attach(true)
  await r.panel.closed('person')
  expect(r.session.isClosed).toBe(true)
  expect(r.session.isShown).toBe(false)
  await r.panel.turnComplete()
  await r.panel.attach(true)
  expect(r.log.opens).toBe(1)
  // The command and the status line open it again.
  expect(await r.panel.command()).toContain('open')
  expect(r.log.opens).toBe(2)
  expect(r.session.isClosed).toBe(false)
})

test('a close by the plugin itself is not the person closing it', async () => {
  const r = rig()
  await r.panel.attach(true)
  await r.panel.closed('plugin')
  expect(r.session.isClosed).toBe(false)
})

test('the command says which of the two it is when nothing opens', async () => {
  const none = rig({ run: 'not-factory', options: { panel: 'command' } })
  await none.panel.attach(true)
  expect(await none.panel.command()).toContain('not a factory')
  expect(none.log.opens).toBe(0)
  const missing = rig({ run: 'no-engine', options: { bf: 'factory-bf' } })
  await missing.panel.attach(true)
  const said = await missing.panel.command()
  expect(said).toContain('factory-bf')
  expect(said).toContain('could not be run')
  expect(missing.log.opens).toBe(0)
})

test('nothing opens by itself in a folder that is no factory', async () => {
  const r = rig({ run: 'not-factory' })
  await r.panel.attach(true)
  expect(r.log.opens).toBe(0)
})

test('presses choose the tab and fold sections, and nothing else', async () => {
  const r = rig({ options: { panel: 'command' } })
  await r.panel.attach(true)
  await r.panel.press('tabs:plan')
  expect(r.choices.tab).toBe('plan')
  await r.panel.press('tab:nothing')
  expect(r.choices.tab).toBe('plan')
  await r.panel.press('fold:pages')
  expect(r.choices.folds['pages']).toBe(true)
  await r.panel.press('fold:pages')
  expect(r.choices.folds['pages']).toBe(false)
  const before = r.log.runs.length
  await r.panel.press('refresh')
  expect(r.log.runs.length).toBe(before + 4)
  await r.panel.press('open-panel')
  expect(r.log.opens).toBe(1)
})

test('the interval reads only while the pane is placed or the status line is drawn', async () => {
  const r = rig({ options: { panel: 'command' } })
  await r.panel.attach(true)
  const base = r.log.runs.length
  await r.panel.tick()
  expect(r.log.runs.length).toBe(base)
  r.log.band = true
  await r.panel.tick()
  expect(r.log.runs.slice(base)).toEqual(['status --json'])
  r.log.band = false
  r.log.pane = true
  await r.panel.tick()
  expect(r.log.runs.length).toBe(base + 2)
})
