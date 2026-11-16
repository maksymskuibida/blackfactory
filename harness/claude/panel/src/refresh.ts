import type { FactoryPanelData } from '../types'
import { buildModel } from './model'
import type { RawReads } from './model'
import { createReader } from './reader'
import type { ReadFailure, RunPort } from './reader'
import { READS, viewArgs } from './reads'

// What the refresh needs from the outside. It runs the engine's reads through `run`, tells time
// through `now` and keeps what it learned through `data`. Nothing else.
export type RefreshPorts = {
  run: RunPort
  now: () => Promise<number>
  bf: () => string
  data: {
    get: () => Promise<FactoryPanelData>
    update: (fn: (d: FactoryPanelData) => FactoryPanelData) => Promise<void>
  }
}

export const EMPTY_DATA: FactoryPanelData = {
  phase: 'loading',
  folder: 'unknown',
  model: null,
  engine: '',
  error: '',
  failed: false,
  readAt: 0,
  events: 0,
  views: {
    waves: [],
    edges: [],
    planState: 'none',
    runState: 'none',
    gateStates: {},
    want: { tab: 'overview', record: null },
    planSections: {},
    gateSections: {},
    receipts: {},
  },
}

export type RefreshOutcome = 'read' | 'unchanged' | 'failed' | 'busy'

// One refresh per session. `full` reads status, progress, records and the index page; `poll` reads
// status alone and reads the rest only when the engine's event count changed.
// A read in flight is shared by every call, so two triggers never run the engine twice at once.
let busy = false

export function createRefresh(ports: RefreshPorts) {
  async function fail(f: ReadFailure): Promise<RefreshOutcome> {
    await ports.data.update(d => {
      if (f.reason === 'not-factory') {
        return { ...d, phase: 'ready', folder: 'not-factory', model: null, error: '', failed: false }
      }
      if (f.reason === 'no-engine' && d.model === null) {
        return { ...d, phase: 'ready', folder: 'no-engine', engine: ports.bf(), model: null, error: f.message, failed: true }
      }
      // A failed read keeps the last good model and says why.
      return { ...d, phase: 'ready', error: f.message, failed: true }
    })
    return 'failed'
  }

  async function readRest(status: unknown, events: number): Promise<RefreshOutcome> {
    const reader = createReader(ports.run, ports.bf())
    const [progress, records, index] = await Promise.all([
      reader.read(READS.progress),
      reader.read(READS.records),
      reader.read(viewArgs('index')),
    ])
    for (const r of [progress, records]) {
      if (!r.ok) return fail(r)
    }
    const raw: RawReads = {
      status,
      progress: progress.ok ? progress.json : null,
      records: records.ok ? records.json : null,
      // The index page only adds the engine's titles: without it the panel still draws.
      index: index.ok ? index.json : null,
    }
    const model = buildModel(raw)
    const at = await ports.now()
    await ports.data.update(d => ({
      ...d,
      phase: 'ready',
      folder: 'factory',
      model,
      error: '',
      failed: false,
      readAt: at,
      events,
    }))
    return 'read'
  }

  function eventsOf(status: unknown): number {
    const n = (status as { events?: unknown } | null)?.events
    return typeof n === 'number' ? n : 0
  }

  async function run(mode: 'full' | 'poll'): Promise<RefreshOutcome> {
    if (busy) return 'busy'
    busy = true
    try {
      const reader = createReader(ports.run, ports.bf())
      const status = await reader.read(READS.status)
      if (!status.ok) return await fail(status)
      const events = eventsOf(status.json)
      if (mode === 'poll') {
        const before = await ports.data.get()
        if (before.model !== null && before.events === events && !before.failed) {
          // Reading the count is a read: the stamp moves, the model stays.
          const at = await ports.now()
          await ports.data.update(d => ({ ...d, readAt: at }))
          return 'unchanged'
        }
      }
      return await readRest(status.json, events)
    } finally {
      busy = false
    }
  }

  return {
    // At session start and attach, on Refresh and when a turn completes.
    full: () => run('full'),
    // On the interval while the pane or the status line is on screen.
    poll: () => run('poll'),
  }
}

export type Refresh = ReturnType<typeof createRefresh>
