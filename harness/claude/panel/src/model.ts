import type {
  FactoryPanelAmendment,
  FactoryPanelCheckpoint,
  FactoryPanelGate,
  FactoryPanelModel,
  FactoryPanelPage,
  FactoryPanelRecord,
  FactoryPanelSegment,
  FactoryPanelStanding,
  FactoryPanelStep,
  FactoryPanelStepId,
  FactoryPanelTrace,
  FactoryPanelUnit,
  FactoryPanelWaiting,
} from '../types'

// What the reads returned, as parsed JSON. Nothing here is trusted to be complete.
export type RawReads = {
  status: unknown
  progress: unknown
  records: unknown
  index?: unknown
}

type Obj = Record<string, unknown>

const ID = /^[A-Z]{1,2}-\d+$/
// Fields of a record that name other records.
const LINK_LISTS = ['links', 'delivers', 'proves', 'after', 'checks', 'depends_on', 'protects']
const LINK_ONES = ['checkpoint', 'unit', 'from', 'to']
const PART_KINDS_OUT = ['unit', 'check', 'checkpoint', 'handoff', 'finding', 'question']

// The kinds each step of the design chain holds.
export const STEP_KINDS: Record<FactoryPanelStepId, string[]> = {
  idea: ['idea'],
  requirements: ['requirement'],
  ui: ['screen', 'journey'],
  architecture: ['component', 'decision', 'target', 'threat', 'markets'],
  plan: ['unit', 'handoff', 'checkpoint', 'check'],
  build: [],
}

// The gate kind of each step (the engine names the requirements gate "requirements").
const STEP_GATE: Record<FactoryPanelStepId, string | null> = {
  idea: 'idea',
  requirements: 'requirements',
  ui: 'ui',
  architecture: 'architecture',
  plan: 'plan',
  build: null,
}

export const STEP_IDS: FactoryPanelStepId[] = ['idea', 'requirements', 'ui', 'architecture', 'plan', 'build']

function obj(v: unknown): Obj {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Obj) : {}
}
function arr(v: unknown): unknown[] {
  return Array.isArray(v) ? v : []
}
function str(v: unknown): string {
  return typeof v === 'string' ? v : ''
}
function num(v: unknown): number {
  return typeof v === 'number' && Number.isFinite(v) ? v : 0
}
function strs(v: unknown): string[] {
  return arr(v).filter((x): x is string => typeof x === 'string')
}

// Splits "U-12" into its letters and its number; null for any other text.
function splitId(id: string): [string, number] | null {
  const at = id.lastIndexOf('-')
  if (at < 1) return null
  const n = Number(id.slice(at + 1))
  return Number.isInteger(n) && id.length > at + 1 ? [id.slice(0, at), n] : null
}

export function naturalCompare(a: string, b: string): number {
  const pa = splitId(a)
  const pb = splitId(b)
  if (pa && pb) {
    if (pa[0] !== pb[0]) return pa[0] < pb[0] ? -1 : 1
    return pa[1] - pb[1]
  }
  return a < b ? -1 : a > b ? 1 : 0
}

function outgoing(fields: Obj): string[] {
  const out: string[] = []
  for (const k of LINK_LISTS) for (const id of strs(fields[k])) if (ID.test(id)) out.push(id)
  for (const k of LINK_ONES) {
    const id = fields[k]
    if (typeof id === 'string' && ID.test(id)) out.push(id)
  }
  return Array.from(new Set(out))
}

function buildRecords(raw: unknown): { records: Record<string, FactoryPanelRecord>; order: string[] } {
  const records: Record<string, FactoryPanelRecord> = {}
  const order: string[] = []
  for (const item of arr(raw)) {
    const f = obj(item)
    const id = str(f.id)
    if (!id) continue
    // The fields kept are the ones the record does not already carry on its own.
    const fields: Obj = {}
    for (const k of Object.keys(f)) {
      if (k !== 'id' && k !== 'kind' && k !== 'title' && k !== 'text' && k !== 'label') fields[k] = f[k]
    }
    records[id] = {
      id,
      kind: str(f.kind),
      title: str(f.title),
      text: str(f.text),
      label: str(f.label),
      fields,
      links: outgoing(f),
      linkedBy: [],
    }
    order.push(id)
  }
  for (const id of order) {
    const rec = records[id]!
    rec.links = rec.links.filter(to => records[to] !== undefined)
    for (const to of rec.links) {
      const target = records[to]!
      if (!target.linkedBy.includes(id)) target.linkedBy.push(id)
    }
  }
  order.sort(naturalCompare)
  return { records, order }
}

// The engine's progress line, split into its segments: "▶ U-4, U-7 building · ✔ U-1 landed".
export function parseProgress(line: string): FactoryPanelSegment[] {
  const out: FactoryPanelSegment[] = []
  for (const part of line.split(' · ')) {
    const m = /^(\S+) ((?:[A-Z]{1,2}-\d+(?: wave \d+)?(?: \(round \d+\))?(?:, )?)+) (.*)$/.exec(part.trim())
    if (!m) continue
    const ids: string[] = []
    let own = 0
    for (const piece of m[2]!.split(', ')) {
      const p = /^([A-Z]{1,2}-\d+(?: wave \d+)?)(?: \(round (\d+)\))?$/.exec(piece.trim())
      if (!p) continue
      ids.push(p[1]!)
      if (p[2]) own = Number(p[2])
    }
    let words = m[3]!
    let round = own
    const r = /^(.*), round (\d+)$/.exec(words)
    if (r) {
      words = r[1]!
      round = Number(r[2])
    }
    out.push({ sign: m[1]!, ids, words, round })
  }
  return out
}

function buildUnits(
  rawUnits: unknown,
  records: Record<string, FactoryPanelRecord>,
): FactoryPanelUnit[] {
  const units: FactoryPanelUnit[] = []
  for (const item of arr(rawUnits)) {
    const f = obj(item)
    const id = str(f.id)
    if (!id) continue
    const rec = records[id]
    const state = str(f.state)
    const rounds = arr(f.review_rounds).length
    const round = state === 'review' ? rounds + 1 : state === 'fixing' ? rounds : 0
    const waiting = f.waiting_for
    units.push({
      id,
      title: rec?.title ?? '',
      state,
      waitingFor: typeof waiting === 'string' && waiting !== '' ? waiting : null,
      round,
      checkpoint: str(rec?.fields.checkpoint),
      delivers: strs(rec?.fields.delivers),
      checks: [],
    })
  }
  units.sort((a, b) => naturalCompare(a.id, b.id))
  return units
}

function buildCheckpoints(
  records: Record<string, FactoryPanelRecord>,
  order: string[],
  units: FactoryPanelUnit[],
  closed: string[],
  fixOpen: string | null,
): FactoryPanelCheckpoint[] {
  const byId = new Map(units.map(u => [u.id, u]))
  const out: FactoryPanelCheckpoint[] = []
  for (const id of order) {
    const rec = records[id]!
    if (rec.kind !== 'checkpoint') continue
    const after = strs(rec.fields.after)
    const isClosed = closed.includes(id)
    const allLanded = after.length > 0 && after.every(u => byId.get(u)?.state === 'landed')
    const started = after.some(u => {
      const s = byId.get(u)?.state
      return s !== undefined && s !== 'planned' && s !== 'ready'
    })
    let phase: FactoryPanelCheckpoint['phase'] = 'planned'
    if (isClosed) phase = 'closed'
    else if (fixOpen === id) phase = 'fixing'
    else if (allLanded) phase = 'verifying'
    else if (started) phase = 'building'
    out.push({ id, title: rec.title, units: after, closed: isClosed, phase })
  }
  return out
}

function buildTraces(
  records: Record<string, FactoryPanelRecord>,
  order: string[],
  units: FactoryPanelUnit[],
): Record<string, FactoryPanelTrace> {
  const traces: Record<string, FactoryPanelTrace> = {}
  for (const id of order) {
    const rec = records[id]!
    if (PART_KINDS_OUT.includes(rec.kind)) continue
    traces[id] = { part: id, units: [], checks: [], checkpoint: null }
  }
  for (const u of units) {
    for (const part of u.delivers) traces[part]?.units.push(u.id)
  }
  for (const id of order) {
    const rec = records[id]!
    if (rec.kind === 'check') {
      for (const part of strs(rec.fields.proves)) traces[part]?.checks.push(id)
    } else if (rec.kind === 'checkpoint') {
      for (const part of strs(rec.fields.proves)) {
        const tr = traces[part]
        if (tr && tr.checkpoint === null) tr.checkpoint = id
      }
    }
  }
  // A unit also carries the checks that prove it.
  const byUnit = new Map(units.map(u => [u.id, u]))
  for (const id of order) {
    const rec = records[id]!
    if (rec.kind === 'check') byUnit.get(str(rec.fields.unit))?.checks.push(id)
  }
  return traces
}

function buildGates(rawGates: unknown, pages: FactoryPanelPage[], titles: Record<string, string>) {
  const byName = new Map(pages.map(p => [p.name, p.url]))
  const gates: FactoryPanelGate[] = []
  for (const item of arr(rawGates)) {
    const f = obj(item)
    const id = str(f.id)
    if (!id) continue
    const kind = str(f.kind)
    const round = num(f.round)
    gates.push({
      id,
      kind,
      round,
      status: str(f.status),
      page: byName.get(`gate-${kind}-r${round}`) ?? byName.get(`gate-${id}`) ?? null,
      title: titles[id] ?? '',
    })
  }
  return gates
}

function buildSteps(
  records: Record<string, FactoryPanelRecord>,
  order: string[],
  gates: FactoryPanelGate[],
  checkpoints: FactoryPanelCheckpoint[],
): FactoryPanelStep[] {
  return STEP_IDS.map(id => {
    const kinds = STEP_KINDS[id]
    const count = order.filter(r => kinds.includes(records[r]!.kind)).length
    const gk = STEP_GATE[id]
    let gate: FactoryPanelStep['gate'] = null
    if (gk) {
      const mine = gates.filter(g => g.kind === gk)
      const last = mine.reduce<FactoryPanelGate | null>((a, g) => (a === null || g.round >= a.round ? g : a), null)
      if (last) gate = { round: last.round, status: last.status }
    }
    const closed = id === 'build' ? checkpoints.filter(k => k.closed).length : 0
    const total = id === 'build' ? checkpoints.length : 0
    return { id, records: count, gate, closed, total }
  })
}

function indexTitles(index: unknown): Record<string, string> {
  const out: Record<string, string> = {}
  for (const s of arr(obj(index).sections)) {
    for (const r of arr(obj(s).rows)) {
      const row = obj(r)
      const id = str(row.id)
      const title = str(row.title)
      if (id && title && out[id] === undefined) out[id] = title
    }
  }
  return out
}

export function buildModel(raw: RawReads): FactoryPanelModel {
  const status = obj(raw.status)
  const { records, order } = buildRecords(raw.records)
  const titles = indexTitles(raw.index)
  const progress = str(obj(raw.progress).line)
  const segments = parseProgress(progress)
  const units = buildUnits(status.units, records)
  const closed = strs(status.checkpoints_closed)
  const fix = obj(obj(status.checkpoint_fix).open)
  const fixOpen = str(fix.checkpoint) || null
  const checkpoints = buildCheckpoints(records, order, units, closed, fixOpen)
  const pages: FactoryPanelPage[] = arr(status.pages)
    .map(p => ({ name: str(obj(p).name), url: str(obj(p).url), at: str(obj(p).at) }))
    .filter(p => p.name !== '' && p.url !== '')
    .sort((a, b) => (a.at < b.at ? 1 : a.at > b.at ? -1 : 0))
  const gates = buildGates(status.gates, pages, titles)
  const amendments: FactoryPanelAmendment[] = arr(status.amendments).map(a => ({
    id: str(obj(a).id),
    ids: strs(obj(a).ids),
    status: str(obj(a).status),
  }))
  const openAmendments = amendments.filter(a => a.status === 'open')
  const run = typeof status.run === 'string' ? status.run : null
  const openGate = [...gates].reverse().find(g => g.status === 'open') ?? null
  const lastApprovedGate = [...gates].reverse().find(g => g.status === 'approved') ?? null
  const landed = units.filter(u => u.state === 'landed').length
  const finished = run !== null && checkpoints.length > 0 && checkpoints.every(k => k.closed)
  const openCheckpoint = checkpoints.find(k => !k.closed)?.id ?? null
  const unitCounts: Record<string, number> = {}
  for (const u of units) unitCounts[u.state] = (unitCounts[u.state] ?? 0) + 1

  const waiting: FactoryPanelWaiting[] = []
  const pageUrl = new Map(pages.map(p => [p.name, p.url]))
  for (const g of gates.filter(g => g.status === 'open')) {
    waiting.push({ kind: 'gate', id: g.id, text: g.title, names: [], page: g.page })
  }
  for (const id of order) {
    const rec = records[id]!
    if (rec.kind !== 'question') continue
    const answered = rec.fields.answer !== undefined && rec.fields.answer !== null && rec.fields.answer !== ''
    if (answered) continue
    waiting.push({
      kind: 'question',
      id,
      text: rec.title,
      names: [],
      page: pageUrl.get('questions') ?? null,
    })
  }
  for (const a of openAmendments) {
    waiting.push({ kind: 'amendment', id: a.id, text: '', names: a.ids, page: null })
  }
  for (const u of units) {
    if (u.state === 'parked' || u.state === 'blocked') {
      waiting.push({
        kind: u.state,
        id: u.id,
        text: u.waitingFor ?? u.title,
        names: [],
        page: null,
      })
    }
  }

  const stuck = units.some(u => u.state === 'parked' || u.state === 'blocked')
  const needsHuman = waiting.some(w => w.kind === 'gate' || w.kind === 'question' || w.kind === 'amendment')
  const working = units.some(u => ['running', 'review', 'fixing', 'reviewed'].includes(u.state))
  const verifying = checkpoints.some(k => k.phase === 'verifying' || k.phase === 'fixing')
  let standing: FactoryPanelStanding
  if (run === null) standing = needsHuman ? 'needs' : 'design'
  else if (needsHuman) standing = 'needs'
  else if (stuck) standing = 'blocked'
  else if (verifying && !working) standing = 'verifying'
  else if (working) standing = 'building'
  else if (finished) standing = 'finished'
  else standing = 'idle'
  // Before a run starts a waiting gate still reads as the design stage with its open gate.
  if (run === null) standing = 'design'

  return {
    project: str(status.project),
    run,
    standing,
    events: num(status.events),
    progress,
    segments,
    records,
    order,
    units,
    unitCounts,
    landed,
    checkpoints,
    openCheckpoint,
    finished,
    traces: buildTraces(records, order, units),
    gates,
    openGate,
    lastApprovedGate,
    amendments,
    openAmendments,
    pages,
    steps: buildSteps(records, order, gates, checkpoints),
    waiting,
    titles,
  }
}
