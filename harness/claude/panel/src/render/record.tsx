import type { FactoryPanelModel, FactoryPanelRecord } from '../../types'
import { ink, size, space } from '../theme'
import { t } from '../strings'
import type { StringKey } from '../strings'
import { noop } from './context'
import type { Ctx, Kids } from './context'
import { button, card, dim, heading, tag } from './parts'
import {
  amendmentOf,
  checkResult,
  idButton,
  idLine,
  idOrDead,
  isStop,
  labelTag,
  pageForRecord,
  readLine,
} from './refs'
import { linkLine } from './parts'

// Screen S-21: one record with every field the engine holds, what it links to and what links to
// it, its trace, and every ID in it a button that opens that record.

const TEXT_FOLD = 12
const LIST_FOLD = 20
const MEASURES = 'The reviewer measures'

const FIELD_HEAD: Record<string, StringKey> = {
  command: 'fieldCommand',
  acceptance: 'fieldAcceptance',
  chosen: 'fieldChosen',
  rejected: 'fieldRejected',
  protects: 'fieldProtects',
  attack: 'fieldAttack',
  mitigation: 'fieldMitigation',
  measure: 'fieldMeasure',
  steps: 'fieldSteps',
  files: 'fieldFiles',
  depends_on: 'fieldDependsOn',
  delivers: 'fieldDelivers',
  checks: 'fieldChecks',
  checkpoint: 'fieldCheckpoint',
  estimate: 'fieldEstimate',
  how: 'fieldHow',
  scope: 'fieldScope',
  unit: 'fieldUnit',
  proves: 'fieldProves',
  after: 'fieldAfter',
  stop: 'fieldStop',
  stop_why: 'fieldStopWhy',
  human_step: 'fieldHumanStep',
  from: 'fieldFrom',
  to: 'fieldTo',
  thing: 'fieldThing',
  contract: 'fieldContract',
  links: 'fieldLinks',
}

// The fields each kind shows first, in the order the pages read them.
const FIRST: Record<string, string[]> = {
  requirement: ['acceptance'],
  decision: ['chosen', 'rejected'],
  threat: ['protects', 'attack', 'mitigation'],
  target: ['measure'],
  journey: ['steps'],
  unit: ['files', 'depends_on', 'delivers', 'checks', 'checkpoint', 'estimate'],
  check: ['command', 'how', 'scope', 'unit', 'proves'],
  checkpoint: ['after', 'proves', 'checks', 'stop', 'stop_why', 'human_step'],
  handoff: ['from', 'to', 'thing', 'contract'],
}

// Fields the engine keeps for itself: they are state, not part of the record's page.
const HIDDEN = new Set(['review_rounds', 'simulated'])

function head(ctx: Ctx, key: string) {
  const k = FIELD_HEAD[key]
  return k !== undefined ? t(k) : key
}

function sentences(line: string): string[] {
  return line.split(/(?<=[.!?])\s+/).filter(s => s !== '')
}

// The text as formatted text: paragraphs, lists, code and the reviewer's sentences set apart.
export function formatted(ctx: Ctx, text: string, place: string, foldKey: string): Kids {
  const { Box, Text } = ctx.ui
  const lines = text.split('\n')
  const open = ctx.choices.folds[foldKey] === true
  const shown = open ? lines : lines.slice(0, TEXT_FOLD)
  const out: Kids = []
  let inCode = false
  shown.forEach((raw, i) => {
    const line = raw.replace(/\s+$/, '')
    const at = `${place}.${i}`
    if (/^```/.test(line.trim())) {
      inCode = !inCode
      return
    }
    if (line.trim() === '') return
    if (inCode) {
      out.push(idLine(ctx, line, at, { mono: true }))
      return
    }
    const bullet = /^\s*[-*]\s+(.*)$/.exec(line)
    const heading1 = /^#{1,6}\s+(.*)$/.exec(line)
    const body = bullet ? bullet[1]! : heading1 ? heading1[1]! : line
    const parts = body.includes(MEASURES) ? sentences(body) : [body]
    const normal: string[] = []
    const apart: string[] = []
    for (const s of parts) (s.startsWith(MEASURES) ? apart : normal).push(s)
    if (normal.length > 0) {
      out.push(
        <Box key={`p${at}`} flexDirection="row" gap={space(1)}>
          {bullet ? <Text>{'-'}</Text> : null}
          {idLine(ctx, normal.join(' '), at, { bold: heading1 !== null })}
        </Box>,
      )
    }
    apart.forEach((s, j) => {
      out.push(
        <Box key={`m${at}.${j}`} flexDirection="column" gap={space(1)} borderStyle="round" borderColor={ink.accent} padding={space(1)}>
          {tag(ctx, 'info', t('reviewerMeasures'))}
          {idLine(ctx, s, `${at}.m${j}`)}
        </Box>,
      )
    })
  })
  if (lines.length > TEXT_FOLD) {
    out.push(button(ctx, `fold:${foldKey}`, open ? t('showFewer') : t('showAll'), noop, { plain: true }))
  }
  return out
}

function valueRows(ctx: Ctx, key: string, value: unknown, place: string): Kids {
  const mono = key === 'command'
  if (typeof value === 'string') return [idLine(ctx, value, place, { mono })]
  if (typeof value === 'number' || typeof value === 'boolean') return [idLine(ctx, String(value), place)]
  if (Array.isArray(value)) {
    return value.map((v, i) =>
      typeof v === 'string' || typeof v === 'number'
        ? idLine(ctx, String(v), `${place}.${i}`, { mono })
        : idLine(ctx, JSON.stringify(v), `${place}.${i}`, { mono: true }),
    )
  }
  if (value && typeof value === 'object') return [idLine(ctx, JSON.stringify(value), place, { mono: true })]
  return []
}

function section(ctx: Ctx, key: string, title: string, rows: Kids) {
  const { Box } = ctx.ui
  const real = rows.filter(r => r !== null)
  if (real.length === 0) return null
  return (
    <Box key={key} flexDirection="column" gap={space(1)}>
      {title !== '' ? titleLine(ctx, title) : null}
      {real}
    </Box>
  )
}

function titleLine(ctx: Ctx, text: string) {
  const { Text } = ctx.ui
  return (
    <Text key={`title:${text}`} bold color={ink.muted} size={size.small}>
      {text}
    </Text>
  )
}

function fieldSections(ctx: Ctx, rec: FactoryPanelRecord): Kids {
  const first = FIRST[rec.kind] ?? []
  const keys = [...first.filter(k => rec.fields[k] !== undefined), ...Object.keys(rec.fields).filter(k => !first.includes(k)).sort()]
  const out: Kids = []
  for (const k of keys) {
    if (HIDDEN.has(k)) continue
    const rows = valueRows(ctx, k, rec.fields[k], `f:${k}`)
    out.push(section(ctx, `field:${k}`, head(ctx, k), rows))
  }
  return out
}

function idList(ctx: Ctx, m: FactoryPanelModel, ids: string[], place: string, foldKey: string) {
  const { Box, Text } = ctx.ui
  const open = ctx.choices.folds[foldKey] === true
  const shown = open ? ids : ids.slice(0, LIST_FOLD)
  const rows: Kids = shown.map(id => (
    <Box key={`row:${place}:${id}`} flexDirection="row" gap={space(2)} flexWrap="wrap" alignItems="center">
      {idButton(ctx, id, `${place}.${id}`)}
      <Text color={ink.muted}>{m.records[id]?.title ?? ''}</Text>
    </Box>
  ))
  if (ids.length > LIST_FOLD) {
    rows.push(button(ctx, `fold:${foldKey}`, open ? t('showFewer') : t('showMore', { count: ids.length - LIST_FOLD }), noop, { plain: true }))
  }
  return rows
}

function unitSection(ctx: Ctx, m: FactoryPanelModel, rec: FactoryPanelRecord) {
  const { Box, Text } = ctx.ui
  const u = m.units.find(x => x.id === rec.id)
  if (!u) return null
  const rows: Kids = [
    <Box key="state" flexDirection="row" gap={space(2)} flexWrap="wrap" alignItems="center">
      <Text color={ink.muted}>{t('state')}</Text>
      {tag(ctx, u.state === 'landed' ? 'ok' : u.state === 'parked' || u.state === 'blocked' ? 'warn' : 'info', u.state)}
    </Box>,
  ]
  if (u.waitingFor !== null) {
    rows.push(
      <Box key="waits" flexDirection="row" gap={space(2)} flexWrap="wrap">
        <Text color={ink.muted}>{t('waitsFor')}</Text>
        {idLine(ctx, u.waitingFor, 'waits')}
      </Box>,
    )
  }
  const checks = u.checks.map(c => {
    const r = checkResult(ctx, c)
    return (
      <Box key={`rc:${c}`} flexDirection="row" gap={space(2)} flexWrap="wrap" alignItems="center">
        {idButton(ctx, c, `rc.${c}`)}
        {tag(ctx, r.tone, r.word)}
      </Box>
    )
  })
  rows.push(readLine(ctx, ctx.data.views.runState))
  if (checks.length === 0) rows.push(ctx.data.views.runState === 'ready' ? dim(ctx, t('noReceipt')) : null)
  else rows.push(titleLine(ctx, t('receipts')), ...checks)
  return card(ctx, null, 'unit', rows)
}

function checkSection(ctx: Ctx, rec: FactoryPanelRecord) {
  const { Box, Text } = ctx.ui
  const r = ctx.data.views.receipts[rec.id]
  const res = checkResult(ctx, rec.id)
  const rows: Kids = [
    <Box key="r" flexDirection="row" gap={space(2)} flexWrap="wrap" alignItems="center">
      <Text color={ink.muted}>{t('receipts')}</Text>
      {tag(ctx, res.tone, res.word)}
      {r ? <Text color={ink.muted}>{r.at.slice(0, 10)}</Text> : null}
    </Box>,
  ]
  rows.push(readLine(ctx, ctx.data.views.runState))
  if (!r && ctx.data.views.runState === 'ready') rows.push(dim(ctx, t('noReceipt')))
  return card(ctx, res.tone === 'crit' ? 'crit' : res.tone === 'ok' ? 'ok' : null, 'check', rows)
}

function traceSection(ctx: Ctx, m: FactoryPanelModel, rec: FactoryPanelRecord) {
  const { Box, Text } = ctx.ui
  const rows: Kids = []
  const line = (key: string, label: string, ids: string[]) => {
    if (ids.length === 0) return
    rows.push(
      <Box key={key} flexDirection="row" gap={space(2)} flexWrap="wrap" alignItems="center">
        <Text color={ink.muted}>{label}</Text>
        {ids.slice(0, LIST_FOLD).map(id => idButton(ctx, id, `${key}.${id}`))}
      </Box>,
    )
  }
  const tr = m.traces[rec.id]
  if (tr) {
    line('tr-units', t('traceDeliveredBy'), tr.units)
    line('tr-checks', t('traceProvenBy'), tr.checks)
    line('tr-cp', t('traceClosedAt'), tr.checkpoint !== null ? [tr.checkpoint] : [])
  } else if (rec.kind === 'unit') {
    const cp = typeof rec.fields.checkpoint === 'string' ? rec.fields.checkpoint : ''
    line('tr-cp', t('traceClosedAt'), cp !== '' ? [cp] : [])
  }
  if (rows.length === 0) return null
  return section(ctx, 'trace', t('trace'), rows)
}

function crumb(ctx: Ctx, id: string) {
  const { Box, Text } = ctx.ui
  const back = ctx.choices.back
  const items: Kids = []
  back.forEach((b, i) => {
    items.push(
      crumbButton(ctx, b, i),
      <Text key={`sep${i}`} color={ink.muted}>
        {'>'}
      </Text>,
    )
  })
  items.push(
    <Text key="here" bold mono>
      {id}
    </Text>,
  )
  return (
    <Box key="crumb" flexDirection="row" flexWrap="wrap" gap={space(1)} alignItems="center">
      {items}
    </Box>
  )
}

function crumbButton(ctx: Ctx, id: string, index: number) {
  const { Button, Text } = ctx.ui
  return (
    <Button plain label={t('crumbLabel', { id })} onPress={noop} key={`crumb:${index}`}>
      <Text mono>{id}</Text>
    </Button>
  )
}

export function recordPage(ctx: Ctx, id: string) {
  const { Box, Text } = ctx.ui
  const m = ctx.data.model!
  const rec = m.records[id]
  const backButton = button(ctx, 'back', t('back'), noop, { hotkey: 'b' })
  if (rec === undefined) {
    return (
      <Box flexDirection="column" gap={space(3)}>
        {backButton}
        {crumb(ctx, id)}
        {dim(ctx, t('recordGone'))}
      </Box>
    )
  }
  const amend = amendmentOf(m, id)
  const url = pageForRecord(m, rec)
  const rows: Kids = []
  rows.push(
    <Box key="head" flexDirection="column" gap={space(1)}>
      <Box flexDirection="row" flexWrap="wrap" gap={space(2)} alignItems="center">
        <Text bold mono size={size.heading}>
          {rec.id}
        </Text>
        <Text color={ink.muted}>{rec.kind}</Text>
        {labelTag(ctx, rec.label, 'label')}
        {amend !== null ? tag(ctx, 'warn', amend, 'amend') : null}
      </Box>
      {rec.title !== '' ? (
        <Text bold size={size.body}>
          {rec.title}
        </Text>
      ) : null}
    </Box>,
  )
  if (rec.text !== '') rows.push(<Box key="text" flexDirection="column" gap={space(2)}>{formatted(ctx, rec.text, 'x', `text:${rec.id}`)}</Box>)
  if (rec.kind === 'unit') rows.push(unitSection(ctx, m, rec))
  if (rec.kind === 'check') rows.push(checkSection(ctx, rec))
  rows.push(...fieldSections(ctx, rec))
  rows.push(section(ctx, 'links-to', t('linksTo'), idList(ctx, m, rec.links, 'lt', `lt:${rec.id}`)))
  rows.push(section(ctx, 'linked-from', t('linkedFrom'), idList(ctx, m, rec.linkedBy, 'lf', `lf:${rec.id}`)))
  rows.push(traceSection(ctx, m, rec))
  if (url !== null) rows.push(linkLine(ctx, url, t('openFullPage')))
  else rows.push(dim(ctx, t('notPublished')))
  return (
    <Box flexDirection="column" gap={space(3)}>
      <Box flexDirection="row" flexWrap="wrap" gap={space(2)} alignItems="center">
        {backButton}
        {crumb(ctx, id)}
      </Box>
      {rows}
    </Box>
  )
}

export { heading, idOrDead }
