import type { FactoryPanelModel, FactoryPanelRecord } from '../../types'
import { ink, labelColors, size, space } from '../theme'
import { t } from '../strings'
import { stepOfKind } from '../views'
import { noop } from './context'
import type { Ctx, Kids } from './context'
import { tag } from './parts'

// Everything the record rows and pages share: IDs as buttons, label tags, the pages the conductor
// recorded and the trace of a record.

const ID_IN_TEXT = /\b([A-Z]{1,2}-\d+)\b/g

// A page address is drawn only when it is an https link.
export function httpsOnly(url: string | null | undefined): string | null {
  return typeof url === 'string' && /^https:\/\/[^\s]+$/.test(url) ? url : null
}

// The latest round of a stage's gate page, among the pages the conductor recorded.
export function stagePage(m: FactoryPanelModel, step: string): string | null {
  let best = -1
  let url: string | null = null
  for (const p of m.pages) {
    const hit = new RegExp(`^gate-${step}-r(\\d+)$`).exec(p.name)
    if (hit && Number(hit[1]) > best) {
      best = Number(hit[1])
      url = p.url
    }
  }
  return httpsOnly(url)
}

export function planPage(m: FactoryPanelModel): string | null {
  return httpsOnly(m.pages.find(p => p.name === 'plan')?.url)
}

// The page that holds a record's row: the stage's gate page for a design part, the plan page for
// a unit, a check or a handoff, checkpoint-<K> for a checkpoint.
export function pageForRecord(m: FactoryPanelModel, rec: FactoryPanelRecord): string | null {
  if (rec.kind === 'unit' || rec.kind === 'check' || rec.kind === 'handoff') return planPage(m)
  if (rec.kind === 'checkpoint') return httpsOnly(m.pages.find(p => p.name === `checkpoint-${rec.id}`)?.url)
  const step = stepOfKind(rec.kind)
  return step === null ? null : stagePage(m, step)
}

// The open amendment that names a record, when there is one.
export function amendmentOf(m: FactoryPanelModel, id: string): string | null {
  return m.openAmendments.find(a => a.ids.includes(id))?.id ?? null
}

export function labelTag(ctx: Ctx, label: string, key?: string) {
  if (label === '') return null
  const c = labelColors(label)
  if (c === null) return tag(ctx, 'neutral', label, key)
  const { Text } = ctx.ui
  return (
    <Text key={key} color={c.fg} backgroundColor={c.bg} bold>
      {' '}
      {label}{' '}
    </Text>
  )
}

// An ID as a button that opens its record. The place makes its key its own on the screen.
export function idButton(ctx: Ctx, id: string, place: string) {
  const { Button, Text } = ctx.ui
  return (
    <Button key={`open:${id}@${place}`} plain label={id} onPress={noop}>
      <Text bold mono>
        {id}
      </Text>
    </Button>
  )
}

// An ID that is no record: dim and not pressable.
export function deadId(ctx: Ctx, id: string, key: string) {
  const { Text } = ctx.ui
  return (
    <Text key={key} color={ink.muted} mono>
      {id}
    </Text>
  )
}

export function idOrDead(ctx: Ctx, id: string, place: string) {
  return ctx.data.model?.records[id] !== undefined ? idButton(ctx, id, place) : deadId(ctx, id, `dead:${id}@${place}`)
}

// A text with every ID in it as a button (or dim when it is no record).
export function idText(ctx: Ctx, text: string, place: string, options?: { mono?: boolean; bold?: boolean }): Kids {
  const { Text } = ctx.ui
  const out: Kids = []
  let last = 0
  let n = 0
  for (const hit of text.matchAll(ID_IN_TEXT)) {
    const at = hit.index ?? 0
    if (at > last) {
      out.push(
        <Text key={`t${n}@${place}`} mono={options?.mono} bold={options?.bold}>
          {text.slice(last, at)}
        </Text>,
      )
    }
    out.push(idOrDead(ctx, hit[1]!, `${place}.${n}`))
    last = at + hit[0].length
    n++
  }
  if (last < text.length) {
    out.push(
      <Text key={`t${n}@${place}`} mono={options?.mono} bold={options?.bold}>
        {text.slice(last)}
      </Text>,
    )
  }
  return out
}

export function idLine(ctx: Ctx, text: string, place: string, options?: { mono?: boolean; bold?: boolean }) {
  const { Box } = ctx.ui
  return (
    <Box key={place} flexDirection="row" flexWrap="wrap" alignItems="center">
      {idText(ctx, text, place, options)}
    </Box>
  )
}

export type Trace = { words: string; ids: string[]; extra: { words: string; ids: string[] } | null; tag: { tone: 'ok' | 'info' | 'warn' | 'crit' | 'neutral'; word: string } | null }

export function checkResult(ctx: Ctx, id: string): { tone: 'ok' | 'crit' | 'neutral'; word: string } {
  const v = ctx.data.views
  // A receipt not read yet, or a read that failed, is never told as "not run".
  if (v.runState === 'failed' && v.receipts[id] === undefined) return { tone: 'neutral', word: t('resultUnread') }
  if (v.runState !== 'ready' && v.receipts[id] === undefined) return { tone: 'neutral', word: t('resultReading') }
  const r = v.receipts[id]
  if (r === undefined) return { tone: 'neutral', word: t('resultNotRun') }
  return r.exit === 0 ? { tone: 'ok', word: t('resultPassed') } : { tone: 'crit', word: t('resultFailed') }
}

// One quiet line for a read that is pending or failed; null when the read answered.
export function readLine(ctx: Ctx, state: string) {
  const { Text } = ctx.ui
  if (state === 'ready') return null
  if (state === 'failed') {
    return (
      <Text key="read-failed" color={ink.bad}>
        {t('readFailedView')}
      </Text>
    )
  }
  return (
    <Text key="read-loading" color={ink.muted} size={size.small}>
      {t('loadingTab')}
    </Text>
  )
}

export function isStop(rec: FactoryPanelRecord | undefined): boolean {
  return rec !== undefined && (rec.fields.stop === true || (typeof rec.fields.human_step === 'string' && rec.fields.human_step !== ''))
}

// The quiet trace of a record, for its row.
export function traceOf(ctx: Ctx, rec: FactoryPanelRecord): Trace | null {
  const m = ctx.data.model!
  if (rec.kind === 'unit') {
    const u = m.units.find(x => x.id === rec.id)
    const cp = typeof rec.fields.checkpoint === 'string' ? rec.fields.checkpoint : ''
    return {
      words: '',
      ids: cp !== '' ? [cp] : [],
      extra: null,
      tag: u ? { tone: u.state === 'landed' ? 'ok' : u.state === 'parked' || u.state === 'blocked' ? 'warn' : 'info', word: u.state } : null,
    }
  }
  if (rec.kind === 'check') {
    const r = checkResult(ctx, rec.id)
    return { words: '', ids: [], extra: null, tag: { tone: r.tone, word: r.word } }
  }
  if (rec.kind === 'checkpoint') {
    const k = m.checkpoints.find(x => x.id === rec.id)
    return {
      words: '',
      ids: [],
      extra: null,
      tag: k ? { tone: k.closed ? 'ok' : 'info', word: k.closed ? t('checkpointClosed') : t('checkpointOpen') } : null,
    }
  }
  const tr = m.traces[rec.id]
  if (!tr) return null
  if (tr.units.length === 0 && tr.checkpoint === null) return null
  return {
    words: t('traceBuiltBy'),
    ids: tr.units.slice(0, 3),
    extra: tr.checkpoint !== null ? { words: t('traceProvedAt'), ids: [tr.checkpoint] } : null,
    tag: null,
  }
}

export function sizeStyle() {
  return { small: size.small, gap: space(2) }
}
