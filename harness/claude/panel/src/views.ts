import type { FactoryPanelData, FactoryPanelModel, FactoryPanelReadState, FactoryPanelReceipt, FactoryPanelViews, FactoryPanelWave } from '../types'
import { STEP_KINDS } from './model'
import { createReader } from './reader'
import type { RunPort } from './reader'
import { viewArgs } from './reads'

// The two reads a tab or a record opens: the engine's plan view (its waves, edges and section
// titles), its run view (the latest receipt of each check) and the gate page of a stage (its
// section titles). They are views of the list in reads.ts; nothing else is run.

type Obj = Record<string, unknown>

function obj(v: unknown): Obj {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Obj) : {}
}
function arr(v: unknown): unknown[] {
  return Array.isArray(v) ? v : []
}
function str(v: unknown): string {
  return typeof v === 'string' ? v : ''
}

function sectionTitles(view: unknown): Record<string, string> {
  const out: Record<string, string> = {}
  for (const s of arr(obj(view).sections)) {
    const id = str(obj(s).id)
    const title = str(obj(s).title)
    if (id !== '' && title !== '') out[id] = title
  }
  return out
}

export function parsePlanView(view: unknown): Pick<FactoryPanelViews, 'waves' | 'edges' | 'planSections'> {
  const waves: FactoryPanelWave[] = []
  const edges: [string, string][] = []
  for (const s of arr(obj(view).sections)) {
    for (const b of arr(obj(s).blocks)) {
      const block = obj(b)
      if (block.type !== 'waves') continue
      for (const w of arr(block.waves)) {
        const wave = obj(w)
        waves.push({
          n: typeof wave.n === 'number' ? wave.n : waves.length + 1,
          units: arr(wave.units).map(u => ({ id: str(obj(u).id), title: str(obj(u).title), state: str(obj(u).state) })).filter(u => u.id !== ''),
        })
      }
      for (const e of arr(block.edges)) {
        const pair = arr(e)
        if (typeof pair[0] === 'string' && typeof pair[1] === 'string') edges.push([pair[0], pair[1]])
      }
    }
    if (waves.length > 0) break
  }
  return { waves, edges, planSections: sectionTitles(view) }
}

export function parseRunView(view: unknown): Record<string, FactoryPanelReceipt> {
  const out: Record<string, FactoryPanelReceipt> = {}
  for (const s of arr(obj(view).sections)) {
    for (const r of arr(obj(s).rows)) {
      for (const b of arr(obj(r).body)) {
        const e = obj(b)
        if (e.type !== 'evidence') continue
        const check = str(e.check)
        if (check === '') continue
        const at = str(e.at)
        const before = out[check]
        if (before === undefined || at >= before.at) {
          out[check] = { check, exit: typeof e.exit === 'number' ? e.exit : 1, at, command: str(e.command) }
        }
      }
    }
  }
  return out
}

// The step a record's kind belongs to, for the gate page that holds its row.
export function stepOfKind(kind: string): string | null {
  for (const [step, kinds] of Object.entries(STEP_KINDS)) if (kinds.includes(kind)) return step
  return null
}

export type ViewPorts = {
  run: RunPort
  bf: () => string
  data: {
    get: () => Promise<FactoryPanelData>
    update: (fn: (d: FactoryPanelData) => FactoryPanelData) => Promise<void>
  }
}

export function createViews(ports: ViewPorts) {
  async function readJson(args: string[]): Promise<unknown | null> {
    const r = await createReader(ports.run, ports.bf()).read(args)
    return r.ok ? r.json : null
  }

  // A read in flight shows as loading unless an earlier answer is still on screen; a failed read
  // says so and keeps what the last good read left.
  async function pending(which: 'planState' | 'runState'): Promise<void> {
    await ports.data.update(d => (d.views[which] === 'ready' ? d : { ...d, views: { ...d.views, [which]: 'loading' } }))
  }

  async function plan(): Promise<void> {
    await pending('planState')
    const json = await readJson(viewArgs('plan'))
    if (json === null) {
      await ports.data.update(d => ({ ...d, views: { ...d.views, planState: 'failed' } }))
      return
    }
    const parsed = parsePlanView(json)
    await ports.data.update(d => ({ ...d, views: { ...d.views, ...parsed, planState: 'ready' } }))
  }

  async function run(): Promise<void> {
    await pending('runState')
    const json = await readJson(viewArgs('run'))
    if (json === null) {
      await ports.data.update(d => ({ ...d, views: { ...d.views, runState: 'failed' } }))
      return
    }
    const receipts = parseRunView(json)
    await ports.data.update(d => ({ ...d, views: { ...d.views, receipts, runState: 'ready' } }))
  }

  async function gate(step: string, model: FactoryPanelModel): Promise<void> {
    const mine = model.gates.filter(g => g.kind === step)
    const last = mine.reduce<(typeof mine)[number] | null>((a, g) => (a === null || g.round >= a.round ? g : a), null)
    if (last === null) return
    const setState = (s: FactoryPanelReadState) =>
      ports.data.update(d => ({ ...d, views: { ...d.views, gateStates: { ...d.views.gateStates, [step]: s } } }))
    const before = (await ports.data.get()).views.gateStates[step]
    if (before !== 'ready') await setState('loading')
    const json = await readJson(viewArgs('gate', last.id))
    if (json === null) {
      await setState('failed')
      return
    }
    const titles = sectionTitles(json)
    await ports.data.update(d => ({
      ...d,
      views: {
        ...d.views,
        gateSections: { ...d.views.gateSections, [step]: titles },
        gateStates: { ...d.views.gateStates, [step]: 'ready' },
      },
    }))
  }

  async function forTab(tab: string): Promise<void> {
    const d = await ports.data.get()
    const model = d.model
    if (model === null) return
    if (tab === 'plan' || tab === 'build') await Promise.all([plan(), run()])
    else if (tab !== 'overview') await gate(tab, model)
  }

  async function forRecord(id: string): Promise<void> {
    const d = await ports.data.get()
    const kind = d.model?.records[id]?.kind
    if (kind === 'unit' || kind === 'check' || kind === 'checkpoint') await run()
  }

  return {
    // A tab opened: what its screen draws from the engine's views.
    async forTab(tab: string): Promise<void> {
      await ports.data.update(d => ({ ...d, views: { ...d.views, want: { tab, record: null } } }))
      await forTab(tab)
    },
    // A record opened: a unit, a check or a checkpoint shows its latest receipts.
    async forRecord(id: string | null): Promise<void> {
      await ports.data.update(d => ({ ...d, views: { ...d.views, want: { ...d.views.want, record: id } } }))
      if (id !== null) await forRecord(id)
    },
    // After a refresh, and when the pane returns: read again what the open tab and record draw from.
    async again(): Promise<void> {
      const want = (await ports.data.get()).views.want
      await forTab(want.tab)
      // The Plan and Build tabs already read the run view: once is enough.
      const tabReadRun = want.tab === 'plan' || want.tab === 'build'
      if (want.record !== null && !tabReadRun) await forRecord(want.record)
    },
  }
}

export type Views = ReturnType<typeof createViews>
