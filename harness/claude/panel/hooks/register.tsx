import { atom, read, update } from 'claude-code'
import type { Register } from 'claude-code'

import type { FactoryPanelChoices, FactoryPanelData, FactoryPanelSession } from '../types'
import { COMMAND, EMPTY_CHOICES, EMPTY_SESSION, PANE_ID, createPanel, normalizeOptions } from '../src/panel'
import type { PanelOptions, PanelPorts } from '../src/panel'
import { EMPTY_DATA } from '../src/refresh'
import { pane } from '../src/render/pane'
import { statusRow } from '../src/render/statusline'
import { t } from '../src/strings'

// Only this file touches the engine interface. Everything else takes ports.

const PLUGIN = 'factory-panel'
const choices = atom({ plugin: 'factory-panel', key: 'choices' } as const, EMPTY_CHOICES, {
  shape: 'choices-1',
})
const data = atom({ plugin: 'factory-panel', key: 'data' } as const, EMPTY_DATA, { shape: 'data-1' })
const session = atom({ plugin: 'factory-panel', key: 'session' } as const, EMPTY_SESSION, {
  shape: 'session-1',
})

// Whether the status line was on screen at its last drawing: drawn, not collapsed, no survey.
// The interval reads it to know whether anything is looking. A reload starts it false and the
// next drawing sets it again.
let isBandDrawn = false
let timer: { cancel: () => void } | null = null

function portsOf($: any, options: PanelOptions): PanelPorts {
  return {
    run: argv => $.process.run(argv, { timeoutMs: 20000 }),
    now: () => $.clock.now(),
    bf: () => options.bf,
    data: {
      get: () => read($, data) as Promise<FactoryPanelData>,
      update: async fn => {
        await update($, data, fn)
      },
    },
    options: () => options,
    session: {
      get: () => read($, session) as Promise<FactoryPanelSession>,
      update: async fn => {
        await update($, session, fn)
      },
    },
    choices: {
      update: async (fn: (c: FactoryPanelChoices) => FactoryPanelChoices) => {
        await update($, choices, fn)
      },
    },
    ui: {
      open: async () => {
        const opened = await $.ui.open({ id: PANE_ID, title: t('tabOverview') })
        return opened.isPlaced === true
      },
      isPlaced: async () => {
        const panes = await $.ui.panes()
        return panes.some((p: { id: string; isPlaced: boolean }) => p.id === PANE_ID && p.isPlaced)
      },
    },
    isBandDrawn: () => isBandDrawn,
  }
}

function panelOf($: any, options: PanelOptions) {
  return createPanel(portsOf($, options))
}

// A hook that fails is skipped; the panel's own reads must never take the session's hooks with them.
async function safely(work: () => Promise<unknown>): Promise<void> {
  try {
    await work()
  } catch {
    // The failure is already in the data the screens draw; nothing else to do.
  }
}

export const register: Register = (on, raw) => {
  const options = normalizeOptions(raw)

  on('session.start', async ($, e, next) => {
    await $.command.register({ name: COMMAND, description: t('openPanelLabel') })
    const surfaces = await $.session.surfaces()
    const isDesktop = e.surface === 'desktop' || surfaces.includes('desktop')
    const panel = panelOf($, options)
    if (timer) timer.cancel()
    timer = null
    if (options.panel !== 'off' && options.pollSeconds > 0) {
      timer = $.clock.every(options.pollSeconds * 1000, () => {
        void safely(() => panelOf($, options).tick())
      })
    }
    await safely(() => panel.attach(isDesktop))
    return next(e)
  })

  on('session.attach', async ($, e, next) => {
    await safely(() => panelOf($, options).attach(e.surface === 'desktop'))
    return next(e)
  })

  on('command.run', { command: 'factory-panel' }, async $ => {
    try {
      const text = await panelOf($, options).command()
      return { text }
    } catch {
      return { text: t('commandFailed') }
    }
  })

  on('ui.close', { id: 'factory-panel' }, async ($, e, next) => {
    await safely(() => panelOf($, options).closed(e.origin.kind))
    return next(e)
  })

  on('turn.complete', async ($, e, next) => {
    await safely(() => panelOf($, options).turnComplete())
    return next(e)
  })

  on('ui.press', { plugin: 'factory-panel' }, async ($, e, next) => {
    await safely(() => panelOf($, options).press(e.element))
    return next(e)
  })

  on('ui.render', { component: 'Pane', requestId: 'factory-panel' }, async ($, e, next) => {
    if (e.surface !== 'desktop' || options.panel === 'off') return next(e)
    const ui = $.ui.resolve(e)
    const ctx = {
      ui,
      cols: e.props.bodyColumns,
      now: await $.clock.now(),
      data: (await read($, data)) as FactoryPanelData,
      choices: (await read($, choices)) as FactoryPanelChoices,
    }
    return pane(ctx)
  })

  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    isBandDrawn = false
    if (e.surface !== 'desktop' || options.panel === 'off' || e.props.hasSurvey) return next(e)
    const s = (await read($, session)) as FactoryPanelSession
    if (s.isShown) return next(e)
    const ui = $.ui.resolve(e)
    const ctx = {
      ui,
      cols: e.props.bodyColumns,
      now: await $.clock.now(),
      data: (await read($, data)) as FactoryPanelData,
      choices: (await read($, choices)) as FactoryPanelChoices,
    }
    const row = statusRow(ctx)
    if (row === null) return next(e)
    isBandDrawn = !e.props.isCollapsed
    const { Box } = ui
    return <Box>{row}</Box>
  })
}
