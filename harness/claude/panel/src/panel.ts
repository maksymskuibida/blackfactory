import type { FactoryPanelChoices, FactoryPanelSession, FactoryPanelTab } from '../types'
import { createRefresh } from './refresh'
import type { RefreshPorts } from './refresh'
import { t } from './strings'
import { createViews } from './views'

export const PANE_ID = 'factory-panel'
export const COMMAND = 'factory-panel'

export type PanelOptions = { panel: 'auto' | 'command' | 'off'; pollSeconds: number; bf: string }

export const DEFAULT_OPTIONS: PanelOptions = { panel: 'auto', pollSeconds: 5, bf: 'bf' }

// The options as the engine hands them over: anything out of range falls back or is clamped.
export function normalizeOptions(raw: unknown): PanelOptions {
  const o = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>
  const panel = o.panel === 'command' || o.panel === 'off' || o.panel === 'auto' ? o.panel : 'auto'
  let poll = typeof o.pollSeconds === 'number' && Number.isFinite(o.pollSeconds) ? o.pollSeconds : 5
  if (poll <= 0) poll = 0
  else if (poll < 3) poll = 3
  const bf = typeof o.bf === 'string' && o.bf.trim() !== '' ? o.bf.trim() : 'bf'
  return { panel, pollSeconds: poll, bf }
}

export const EMPTY_SESSION: FactoryPanelSession = {
  isDesktop: false,
  isClosed: false,
  wasOpened: false,
  isShown: false,
}

export const EMPTY_CHOICES: FactoryPanelChoices = {
  tab: 'overview',
  open: null,
  back: [],
  folds: {},
  search: '',
}

export type PanelPorts = RefreshPorts & {
  options: () => PanelOptions
  session: {
    get: () => Promise<FactoryPanelSession>
    update: (fn: (s: FactoryPanelSession) => FactoryPanelSession) => Promise<void>
  }
  choices: {
    update: (fn: (c: FactoryPanelChoices) => FactoryPanelChoices) => Promise<void>
  }
  ui: {
    // Opens the pane; whether it was placed.
    open: () => Promise<boolean>
    isPlaced: () => Promise<boolean>
  }
  // Whether the status line is on screen now: drawn, not collapsed, no survey holding the band.
  isBandDrawn: () => boolean
}

const TABS: FactoryPanelTab[] = ['overview', 'idea', 'requirements', 'ui', 'architecture', 'plan', 'build']

export function createPanel(p: PanelPorts) {
  const refresh = createRefresh(p)
  const views = createViews(p)

  async function active(): Promise<boolean> {
    const s = await p.session.get()
    return s.isDesktop && p.options().panel !== 'off'
  }

  async function maybeOpen(): Promise<void> {
    if (p.options().panel !== 'auto') return
    const s = await p.session.get()
    if (!s.isDesktop || s.isClosed || s.wasOpened) return
    const d = await p.data.get()
    // The pane opens by itself only after a read succeeded in a factory folder.
    if (d.folder !== 'factory' || d.model === null || d.failed) return
    await p.session.update(x => ({ ...x, wasOpened: true }))
    const placed = await p.ui.open()
    await p.session.update(x => ({ ...x, isShown: placed }))
  }

  return {
    refresh,

    // The session started or a client attached. `isDesktop` says where it draws.
    async attach(isDesktop: boolean): Promise<void> {
      if (isDesktop) await p.session.update(s => ({ ...s, isDesktop: true }))
      if (!(await active())) return
      await refresh.full()
      await views.again()
      await maybeOpen()
    },

    async turnComplete(): Promise<void> {
      if (!(await active())) return
      await refresh.full()
      await views.again()
      await maybeOpen()
    },

    // The command: it answers in words, and opens the pane when it can.
    async command(): Promise<string> {
      const s = await p.session.get()
      if (!s.isDesktop) return t('desktopOnly')
      if (p.options().panel === 'off') return t('commandOff')
      await refresh.full()
      await views.again()
      const d = await p.data.get()
      if (d.folder === 'not-factory') return t('commandNotFactory')
      if (d.folder === 'no-engine') return t('commandNoEngine', { bf: p.options().bf })
      await p.session.update(x => ({ ...x, isClosed: false, wasOpened: true }))
      const placed = await p.ui.open()
      await p.session.update(x => ({ ...x, isShown: placed }))
      return placed ? t('commandOpened') : t('commandUnplaced')
    },

    // The pane closed. Only the person's close keeps it closed for the session.
    async closed(origin: string): Promise<void> {
      await p.session.update(s => ({ ...s, isShown: false, isClosed: s.isClosed || origin === 'person' }))
    },

    // Whether the interval should read now: the pane is placed, or the status line is on screen.
    async shouldPoll(): Promise<boolean> {
      if (p.options().pollSeconds <= 0) return false
      if (!(await active())) return false
      const d = await p.data.get()
      if (d.folder !== 'factory') return false
      return (await p.ui.isPlaced()) || p.isBandDrawn()
    },

    async tick(): Promise<void> {
      if (!(await this.shouldPoll())) return
      // The event count changed: the views the open tab and record draw from are read again too.
      if ((await refresh.poll()) === 'read') await views.again()
    },

    // The text of the search field changed.
    async search(value: string): Promise<void> {
      await p.choices.update(c => ({ ...c, search: value }))
    },

    // A press on one of the panel's buttons, by its key.
    async press(key: string): Promise<void> {
      if (key === 'refresh') {
        await refresh.full()
        await views.again()
      } else if (key === 'open-panel') {
        await p.session.update(s => ({ ...s, isClosed: false, wasOpened: true }))
        const placed = await p.ui.open()
        await p.session.update(s => ({ ...s, isShown: placed }))
        // The pane returns on the tab and the record it left: read what they draw from again.
        await views.again()
      } else if (key.startsWith('tab:') || key.startsWith('tabs:')) {
        const tab = key.slice(key.indexOf(':') + 1) as FactoryPanelTab
        if (TABS.includes(tab)) {
          await p.choices.update(c => ({ ...c, tab, open: null, back: [], search: '' }))
          await views.forTab(tab)
        }
      } else if (key.startsWith('open:')) {
        // A press on an ID: the record opens and the one left is kept for the way back.
        // The key may end in @ and a place, so one ID can be a button in many places.
        const id = key.slice(5).split('@')[0]!
        await p.choices.update(c => ({ ...c, open: id, back: c.open !== null && c.open !== id ? [...c.back, c.open] : c.back }))
        await views.forRecord(id)
      } else if (key === 'back') {
        let next = null as string | null
        await p.choices.update(c => {
          next = c.back[c.back.length - 1] ?? null
          return { ...c, open: next, back: c.back.slice(0, -1) }
        })
        await views.forRecord(next)
      } else if (key.startsWith('crumb:')) {
        const at = Number(key.slice(6))
        let next = null as string | null
        await p.choices.update(c => {
          if (!(Number.isInteger(at) && at >= 0 && at < c.back.length)) return c
          next = c.back[at]!
          return { ...c, open: next, back: c.back.slice(0, at) }
        })
        if (next !== null) await views.forRecord(next)
      } else if (key === 'search-clear') {
        await p.choices.update(c => ({ ...c, search: '' }))
      } else if (key.startsWith('fold:')) {
        const id = key.slice(5)
        await p.choices.update(c => ({ ...c, folds: { ...c.folds, [id]: !c.folds[id] } }))
      }
    },
  }
}

export type Panel = ReturnType<typeof createPanel>
