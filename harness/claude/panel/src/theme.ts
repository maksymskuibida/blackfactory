import { tokens } from './tokens'
import type { ToneName } from './tokens'

// Text, background and accent follow the app's own theme colours: the API does not tell the
// panel whether the app is light or dark. Tags keep the tokens' tones and carry their own
// background, so they read in either look.
export const ink = {
  text: 'text',
  muted: 'inactive',
  accent: 'suggestion',
  warn: 'warning',
  bad: 'error',
  good: 'success',
} as const

export type Tone = { bg: string; fg: string; dot: string }

export function tone(name: ToneName): Tone {
  return tokens.tones[name].light
}

export function toneOfStanding(standing: string, hasWait: boolean): ToneName {
  if (standing === 'needs' || standing === 'blocked') return 'warn'
  if (standing === 'finished') return 'ok'
  if (standing === 'design') return hasWait ? 'warn' : 'info'
  return 'info'
}

// The panel's type scale is the pages' scale; the desktop draws five named sizes.
export function textSize(px: number): 'caption' | 'footnote' | 'body' | 'heading' | 'title' {
  const t = tokens.type
  if (px >= t['h1']) return 'title'
  if (px >= t['fact']) return 'heading'
  if (px >= t['base']) return 'body'
  if (px >= t['sm']) return 'footnote'
  return 'caption'
}

export const size = {
  title: textSize(tokens.type['h1']),
  heading: textSize(tokens.type['fact']),
  body: textSize(tokens.type['base']),
  small: textSize(tokens.type['sm']),
}

const SPACES = ['xs', 'sm', 'md', 'lg', 'xl'] as const
// The pages' spacing steps, in order, as the desktop's named spaces.
export function space(step: 1 | 2 | 3 | 4 | 5): (typeof SPACES)[number] {
  const keys = Object.keys(tokens.space)
  const i = Math.min(keys.indexOf(String(step)), SPACES.length - 1)
  return SPACES[Math.max(i, 0)]!
}

// A bar of landed units over all units, one picture carrying both themes.
export function meterSource(landed: number, total: number, width: number, height: number): string {
  const share = total > 0 ? Math.max(0, Math.min(1, landed / total)) : 0
  const done = Math.round(width * share)
  const r = tokens.radius['tag']
  const l = tokens.light
  const d = tokens.dark
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}">` +
    `<style>.track{fill:${l['line']}}.done{fill:${l['accent']}}` +
    `@media (prefers-color-scheme: dark){.track{fill:${d['line']}}.done{fill:${d['accent']}}}</style>` +
    `<rect class="track" x="0" y="0" width="${width}" height="${height}" rx="${r}"/>` +
    (done > 0 ? `<rect class="done" x="0" y="0" width="${done}" height="${height}" rx="${r}"/>` : '') +
    `</svg>`
  )
}

// The tones of the label tags of the pages: critical, high, medium, low, core, main and minor.
const LABEL_KEY: Record<string, string> = {
  critical: 'crit',
  high: 'high',
  medium: 'med',
  low: 'low',
  core: 'core',
  main: 'main',
  minor: 'minor',
}

export function labelColors(label: string): { bg: string; fg: string } | null {
  const key = LABEL_KEY[label]
  if (key === undefined) return null
  const l = tokens.light as Record<string, string>
  const bg = l[`${key}-bg`]
  const fg = l[`${key}-fg`]
  return bg && fg ? { bg, fg } : null
}

// The unit states the plan map tells apart, each with the pages' colour pair it is filled with.
export type StateGroup = 'planned' | 'building' | 'review' | 'fixing' | 'landed' | 'parked'

export const STATE_GROUPS: StateGroup[] = ['planned', 'building', 'review', 'fixing', 'landed', 'parked']

const STATE_KEY: Record<StateGroup, string> = {
  planned: 'minor',
  building: 'med',
  review: 'main',
  fixing: 'high',
  landed: 'ok',
  parked: 'crit',
}

export function groupOfState(state: string): StateGroup {
  if (state === 'running') return 'building'
  if (state === 'review' || state === 'reviewed') return 'review'
  if (state === 'fixing') return 'fixing'
  if (state === 'landed') return 'landed'
  if (state === 'parked' || state === 'blocked') return 'parked'
  return 'planned'
}

export function stateColors(group: StateGroup, theme: 'light' | 'dark'): { bg: string; fg: string } {
  const set = tokens[theme] as Record<string, string>
  const key = STATE_KEY[group]
  return { bg: set[`${key}-bg`] ?? set['line'] ?? '#888888', fg: set[`${key}-fg`] ?? set['fg'] ?? '#000000' }
}

// The class names and the colours of a picture, both themes in one style block.
export function pictureStyle(): string {
  const rules = (theme: 'light' | 'dark') => {
    const set = tokens[theme] as Record<string, string>
    let css = `.line{stroke:${set['muted']};fill:none}.frame{fill:${set['surface']};stroke:${set['line']}}` +
      `.ink{fill:${set['fg']}}.sub{fill:${set['muted']}}.warn{fill:${set['crit-bg']};stroke:${set['crit-fg']}}`
    for (const g of STATE_GROUPS) {
      const c = stateColors(g, theme)
      css += `.s-${g}{fill:${c.bg};stroke:${c.fg}}`
    }
    return css
  }
  return `<style>text{font-family:${tokens.fonts.body.replace(/"/g, "'")}}${rules('light')}@media (prefers-color-scheme: dark){${rules('dark')}}</style>`
}
