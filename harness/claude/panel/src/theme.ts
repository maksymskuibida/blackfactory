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
