import type { FactoryPanelModel, FactoryPanelViews } from '../types'
import { groupOfState, pictureStyle } from './theme'

// The pictures of the panel are drawn here from plain data: no element, no word. The words a picture
// holds are the engine's own (IDs, titles, labels, states); the legend and the text form are drawn
// by the screens from the panel's strings.

// A picture over this many characters is not drawn.
export const PICTURE_LIMIT = 131072
// Above this many boxes there is no picture, only the text form.
export const BOX_LIMIT = 60

export type Layer = { n: number; ids: string[] }

export type ComponentScheme = {
  layers: Layer[]
  // [from, to]: the first component links to the second.
  edges: [string, string][]
  // Links of a component to a component that is not a record: [from, missing id].
  missing: [string, string][]
}

const COMPONENT_ID = /^C-\d+$/

function strs(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []
}

// The components in layers: those nothing links to at the top, each next layer below the ones that
// link to it. A link back to a component above is a cycle and is drawn as a line like any other.
export function componentScheme(m: FactoryPanelModel): ComponentScheme {
  const comps = m.order.filter(id => m.records[id]!.kind === 'component')
  const set = new Set(comps)
  const edges: [string, string][] = []
  const missing: [string, string][] = []
  for (const id of comps) {
    const seen = new Set<string>()
    for (const to of strs(m.records[id]!.fields.links)) {
      if (to === id || seen.has(to)) continue
      seen.add(to)
      if (set.has(to)) edges.push([id, to])
      else if (COMPONENT_ID.test(to) && m.records[to] === undefined) missing.push([id, to])
    }
  }
  const depth = new Map<string, number>()
  for (const id of comps) depth.set(id, 0)
  // Longest path from the top, with a pass limit so a cycle ends.
  for (let pass = 0; pass < comps.length; pass++) {
    let moved = false
    for (const [a, b] of edges) {
      const want = depth.get(a)! + 1
      if (want > depth.get(b)! && want < comps.length) {
        depth.set(b, want)
        moved = true
      }
    }
    if (!moved) break
  }
  const byLayer = new Map<number, string[]>()
  for (const id of comps) {
    const d = depth.get(id)!
    const list = byLayer.get(d) ?? []
    list.push(id)
    byLayer.set(d, list)
  }
  const layers = [...byLayer.keys()]
    .sort((a, b) => a - b)
    .map((n, i) => ({ n: i + 1, ids: byLayer.get(n)! }))
  return { layers, edges, missing }
}

export type PlanMap = {
  waves: { n: number; ids: string[] }[]
  edges: [string, string][]
  units: number
}

export function planMap(views: Pick<FactoryPanelViews, 'waves' | 'edges'>): PlanMap {
  const waves = views.waves.map(w => ({ n: w.n, ids: w.units.map(u => u.id) }))
  const known = new Set(waves.flatMap(w => w.ids))
  const edges = views.edges.filter(([a, b]) => known.has(a) && known.has(b))
  return { waves, edges, units: known.size }
}

export type Picture = { source: string; width: number; height: number; size: number } | null

function esc(s: string): string {
  return s.replace(/[&<>"]/g, c => (c === '&' ? '&amp;' : c === '<' ? '&lt;' : c === '>' ? '&gt;' : '&quot;'))
}

function cut(s: string, n: number): string {
  return s.length <= n ? s : `${s.slice(0, Math.max(1, n - 1))}~`
}

type Box = { id: string; x: number; y: number; w: number; h: number }

function wrap(width: number, height: number, body: string): { source: string; width: number; height: number } {
  const source =
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">` +
    `${pictureStyle()}${body}</svg>`
  return { source, width, height }
}

function lineOf(a: Box, b: Box): string {
  const x1 = a.x + a.w / 2
  const y1 = a.y + a.h
  const x2 = b.x + b.w / 2
  const y2 = b.y
  return `<line class="line" x1="${x1}" y1="${y1}" x2="${x2}" y2="${y2}"/>`
}

// The component scheme as one picture; null when there is nothing to draw, too many boxes or too big.
export function componentPicture(m: FactoryPanelModel, scheme: ComponentScheme): Picture {
  const count = scheme.layers.reduce((n, l) => n + l.ids.length, 0)
  if (count === 0 || count > BOX_LIMIT) return null
  const w = 150
  const h = 46
  const gx = 16
  const gy = 40
  const widest = Math.max(...scheme.layers.map(l => l.ids.length))
  const total = widest * (w + gx) + gx
  const boxes = new Map<string, Box>()
  let body = ''
  scheme.layers.forEach((layer, row) => {
    const rowWidth = layer.ids.length * (w + gx) - gx
    const left = (total - rowWidth) / 2
    layer.ids.forEach((id, i) => {
      boxes.set(id, { id, x: left + i * (w + gx), y: gy / 2 + row * (h + gy), w, h })
    })
  })
  for (const [a, b] of scheme.edges) body += lineOf(boxes.get(a)!, boxes.get(b)!)
  for (const box of boxes.values()) {
    const r = m.records[box.id]!
    body +=
      `<rect class="frame" x="${box.x}" y="${box.y}" width="${box.w}" height="${box.h}" rx="8"/>` +
      `<text class="ink" x="${box.x + 8}" y="${box.y + 16}" font-size="12" font-weight="600">${esc(box.id)}</text>` +
      `<text class="sub" x="${box.x + box.w - 8}" y="${box.y + 16}" font-size="10" text-anchor="end">${esc(r.label)}</text>` +
      `<text class="ink" x="${box.x + 8}" y="${box.y + 33}" font-size="11">${esc(cut(r.title, 24))}</text>`
  }
  const height = scheme.layers.length * (h + gy)
  const pic = wrap(total, height, body)
  return pic.source.length > PICTURE_LIMIT ? null : { ...pic, size: pic.source.length }
}

// The plan map as one picture: the waves top to bottom, units filled by state, lines from a unit to
// the units that depend on it, and a block per checkpoint.
export type PlanWords = { closed: string; open: string; stop: string }

export function planPicture(m: FactoryPanelModel, map: PlanMap, words: PlanWords): Picture {
  if (map.units === 0 || map.units > BOX_LIMIT) return null
  const stateOf = new Map(m.units.map(u => [u.id, u]))
  const w = 70
  const h = 40
  const gx = 10
  const gy = 34
  const widest = Math.max(1, ...map.waves.map(x => x.ids.length), m.checkpoints.length * 2)
  const total = widest * (w + gx) + gx
  const boxes = new Map<string, Box>()
  map.waves.forEach((wave, row) => {
    const rowWidth = wave.ids.length * (w + gx) - gx
    const left = (total - rowWidth) / 2
    wave.ids.forEach((id, i) => {
      boxes.set(id, { id, x: left + i * (w + gx), y: gy / 2 + row * (h + gy), w, h })
    })
  })
  let body = ''
  for (const [a, b] of map.edges) body += lineOf(boxes.get(a)!, boxes.get(b)!)
  for (const box of boxes.values()) {
    const u = stateOf.get(box.id)
    const group = groupOfState(u?.state ?? 'planned')
    body +=
      `<rect class="s-${group}" x="${box.x}" y="${box.y}" width="${box.w}" height="${box.h}" rx="6"/>` +
      `<text class="ink" x="${box.x + 6}" y="${box.y + 16}" font-size="12" font-weight="600">${esc(box.id)}</text>` +
      `<text class="sub" x="${box.x + 6}" y="${box.y + 32}" font-size="10">${esc(u?.checkpoint ?? '')}</text>`
  }
  const rowsBottom = map.waves.length * (h + gy)
  const kw = 150
  const kh = 30
  m.checkpoints.forEach((k, i) => {
    const per = Math.max(1, Math.floor((total - gx) / (kw + gx)))
    const row = Math.floor(i / per)
    const col = i % per
    const x = gx + col * (kw + gx)
    const y = rowsBottom + row * (kh + 12)
    const rec = m.records[k.id]
    const stops = rec !== undefined && (rec.fields.stop === true || (typeof rec.fields.human_step === 'string' && rec.fields.human_step !== ''))
    body +=
      `<rect class="frame" x="${x}" y="${y}" width="${kw}" height="${kh}" rx="6"/>` +
      `<text class="ink" x="${x + 8}" y="${y + 19}" font-size="12" font-weight="600">${esc(k.id)}</text>` +
      `<text class="sub" x="${x + 44}" y="${y + 19}" font-size="10">${esc([k.closed ? words.closed : words.open, stops ? words.stop : ''].filter(x => x !== '').join(' · '))}</text>`
  })
  const per = Math.max(1, Math.floor((total - gx) / (kw + gx)))
  const blockRows = Math.ceil(m.checkpoints.length / per)
  const pic = wrap(total, rowsBottom + blockRows * (kh + 12) + 4, body)
  return pic.source.length > PICTURE_LIMIT ? null : { ...pic, size: pic.source.length }
}
