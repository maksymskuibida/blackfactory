import { en } from './en'
import type { StringKey } from './en'

export type { StringKey }

// The one function every screen reads the panel's own words through. A missing or empty text
// gives an empty string, and the caller hides the element; no other text takes its place.
export function t(key: StringKey, values?: Record<string, string | number>): string {
  const text: string = en[key] ?? ''
  if (text === '') return ''
  if (!values) return text
  return text.replace(/\{(\w+)\}/g, (whole, name: string) => {
    const v = values[name]
    return v === undefined ? '' : String(v)
  })
}
