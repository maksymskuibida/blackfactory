import type { FactoryPanelRecord, FactoryPanelStepId } from '../../types'
import { STEP_KINDS } from '../model'
import { ink, size, space } from '../theme'
import { t } from '../strings'
import type { StringKey } from '../strings'
import { noop, widthOf } from './context'
import type { Ctx, Kids } from './context'
import { button, dim, heading, tag } from './parts'
import { amendmentOf, idButton, labelTag, readLine, stagePage, traceOf } from './refs'
import { formatted } from './record'
import { componentSchemeView, planMapView } from './schemes'
import { linkLine } from './parts'
import { stepName } from './overview'

// Screen S-20: a stage's tab, its records by kind, one row each, with a search and a fold.

const FOLD = 20

const KIND_WORD: Record<string, StringKey> = {
  idea: 'kindIdea',
  requirement: 'kindRequirement',
  screen: 'kindScreen',
  journey: 'kindJourney',
  component: 'kindComponent',
  decision: 'kindDecision',
  target: 'kindTarget',
  threat: 'kindThreat',
  markets: 'kindMarkets',
  unit: 'kindUnit',
  handoff: 'kindHandoff',
  checkpoint: 'kindCheckpoint',
  check: 'kindCheck',
}

const PLAN_SECTION: Record<string, string> = { unit: 'units', handoff: 'handoffs', checkpoint: 'checkpoints' }

// The section title of a kind: the engine's gate or plan page gives one, the panel's words fill the rest.
export function sectionTitle(ctx: Ctx, step: FactoryPanelStepId, kind: string): string {
  const v = ctx.data.views
  const fromGate = v.gateSections[step]?.[kind]
  if (fromGate) return fromGate
  const planKey = PLAN_SECTION[kind]
  const fromPlan = planKey !== undefined ? v.planSections[planKey] : undefined
  if (fromPlan) return fromPlan
  const k = KIND_WORD[kind]
  return k !== undefined ? t(k) : kind
}

export function matches(rec: FactoryPanelRecord, search: string): boolean {
  const q = search.trim().toLowerCase()
  if (q === '') return true
  return rec.id.toLowerCase().includes(q) || rec.title.toLowerCase().includes(q)
}

function traceView(ctx: Ctx, rec: FactoryPanelRecord) {
  const { Box, Text } = ctx.ui
  const tr = traceOf(ctx, rec)
  if (tr === null) return null
  return (
    <Box key={`trace:${rec.id}`} flexDirection="row" flexWrap="wrap" gap={space(1)} alignItems="center">
      {tr.tag ? tag(ctx, tr.tag.tone, tr.tag.word) : null}
      {tr.words !== '' ? <Text color={ink.muted}>{tr.words}</Text> : null}
      {tr.ids.map(id => idButton(ctx, id, `tr:${rec.id}`))}
      {tr.extra ? <Text color={ink.muted}>{tr.extra.words}</Text> : null}
      {tr.extra ? tr.extra.ids.map(id => idButton(ctx, id, `tr:${rec.id}`)) : null}
    </Box>
  )
}

function row(ctx: Ctx, rec: FactoryPanelRecord, width: number | undefined) {
  const { Box, Button, Text } = ctx.ui
  const narrow = widthOf(ctx.cols) === 'narrow'
  const amend = amendmentOf(ctx.data.model!, rec.id)
  const c = ctx.data.model!
  void c
  return (
    <Box key={`row:${rec.id}`} flexDirection={narrow ? 'column' : 'row'} justifyContent="space-between" gap={space(2)} width={width}>
      <Button key={`open:${rec.id}`} plain label={`${rec.id} ${rec.title}`} onPress={noop}>
        <Text bold mono>
          {rec.id}
        </Text>{' '}
        {rec.label !== '' ? labelTag(ctx, rec.label) : null}
        {rec.label !== '' ? ' ' : null}
        {amend !== null ? tag(ctx, 'warn', amend) : null}
        {amend !== null ? ' ' : null}
        <Text>{rec.title}</Text>
      </Button>
      {traceView(ctx, rec)}
    </Box>
  )
}

function search(ctx: Ctx) {
  const { Input } = ctx.ui
  const on = (v: string) => ctx.onSearch?.(v)
  return (
    <Input
      key="search"
      label={t('searchLabel') || undefined}
      placeholder={t('searchPlaceholder') || undefined}
      value={ctx.choices.search}
      onInput={on}
      onSubmit={on}
    />
  )
}

function kindSection(ctx: Ctx, step: FactoryPanelStepId, kind: string): { node: Kids[number]; count: number } {
  const { Box } = ctx.ui
  const m = ctx.data.model!
  const q = ctx.choices.search
  const all = m.order.filter(id => m.records[id]!.kind === kind)
  const ids = all.filter(id => matches(m.records[id]!, q))
  const foldKey = `sec:${step}:${kind}`
  const open = ctx.choices.folds[foldKey] === true
  const shown = open ? ids : ids.slice(0, FOLD)
  const wide = widthOf(ctx.cols) === 'wide'
  const width = wide ? Math.max(20, Math.floor(ctx.cols / 2) - 2) : undefined
  const rows: Kids = [heading(ctx, sectionTitle(ctx, step, kind))]
  if (all.length === 0) rows.push(dim(ctx, t('kindEmpty')))
  rows.push(
    <Box key="rows" flexDirection={wide ? 'row' : 'column'} flexWrap="wrap" gap={space(2)}>
      {shown.map(id => row(ctx, m.records[id]!, width))}
    </Box>,
  )
  if (ids.length > FOLD) {
    rows.push(button(ctx, `fold:${foldKey}`, open ? t('showFewer') : t('showMore', { count: ids.length - FOLD }), noop, { plain: true }))
  }
  return {
    count: ids.length,
    node: (
      <Box key={`sec:${kind}`} flexDirection="column" gap={space(2)}>
        {rows}
      </Box>
    ),
  }
}

export function stageTab(ctx: Ctx, step: FactoryPanelStepId) {
  const { Box, Text } = ctx.ui
  const m = ctx.data.model!
  const rows: Kids = [
    <Text key="title" bold size={size.title}>
      {stepName(step)}
    </Text>,
  ]
  const kinds = step === 'build' ? ['checkpoint'] : STEP_KINDS[step]
  if (step === 'idea') {
    for (const id of m.order.filter(r => m.records[r]!.kind === 'idea')) {
      const rec = m.records[id]!
      rows.push(
        <Box key={`idea:${id}`} flexDirection="column" gap={space(2)}>
          {rec.title !== '' ? (
            <Text bold size={size.heading}>
              {rec.title}
            </Text>
          ) : null}
          {formatted(ctx, rec.text, `idea.${id}`, `text:${id}`)}
        </Box>,
      )
    }
    if (!m.order.some(r => m.records[r]!.kind === 'idea')) rows.push(dim(ctx, t('ideaEmpty')))
  }
  // The engine's gate page gives the section titles: say so while it is read or when it failed.
  const gateState = ctx.data.views.gateStates[step]
  if (gateState !== undefined) rows.push(readLine(ctx, gateState))
  rows.push(search(ctx))
  if (step === 'architecture') rows.push(componentSchemeView(ctx))
  if (step === 'plan' || step === 'build') rows.push(planMapView(ctx))
  let total = 0
  const sections: Kids = []
  for (const kind of kinds) {
    const s = kindSection(ctx, step, kind)
    total += s.count
    if (ctx.choices.search.trim() === '' || s.count > 0) sections.push(s.node)
  }
  if (ctx.choices.search.trim() !== '' && total === 0) {
    rows.push(
      <Box key="nomatch" flexDirection="column" gap={space(2)}>
        {dim(ctx, t('searchNone'))}
        {button(ctx, 'search-clear', t('searchClear'), noop)}
      </Box>,
    )
  } else {
    rows.push(...sections)
  }
  const gateLink = step === 'build' ? null : stagePage(m, step)
  if (gateLink !== null) rows.push(linkLine(ctx, gateLink, t('stageLink')))
  else if (step !== 'build') rows.push(dim(ctx, t('notPublished')))
  return (
    <Box flexDirection="column" gap={space(3)}>
      {rows}
    </Box>
  )
}
