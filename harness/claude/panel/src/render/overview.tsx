import type {
  FactoryPanelModel,
  FactoryPanelStep,
  FactoryPanelWaiting,
} from '../../types'
import { ink, meterSource, size, space } from '../theme'
import { t } from '../strings'
import type { StringKey } from '../strings'
import type { ToneName } from '../tokens'
import { VERSION } from '../version'
import { noop, widthOf } from './context'
import type { Ctx, Kids } from './context'
import { button, card, dim, heading, stamp, tag } from './parts'

const STEP_NAME: Record<string, StringKey> = {
  idea: 'stepIdea',
  requirements: 'stepRequirements',
  ui: 'stepUi',
  architecture: 'stepArchitecture',
  plan: 'stepPlan',
  build: 'stepBuild',
}

const WAIT_WORD: Record<string, StringKey> = {
  gate: 'waitGate',
  question: 'waitQuestion',
  amendment: 'waitAmendment',
  parked: 'waitParked',
  blocked: 'waitBlocked',
}

export function stepName(id: string): string {
  const key = STEP_NAME[id]
  return key ? t(key) : ''
}

// The state of a gate in the panel's words and the tone of its tag.
export function gateState(status: string): { word: string; tone: ToneName } | null {
  if (status === 'approved') return { word: t('gateApproved'), tone: 'ok' }
  if (status === 'open') return { word: t('gateOpen'), tone: 'warn' }
  if (status === 'rejected') return { word: t('gateChanges'), tone: 'info' }
  return null
}

function header(ctx: Ctx, m: FactoryPanelModel | null) {
  const { Box, Text } = ctx.ui
  const narrow = widthOf(ctx.cols) === 'narrow'
  return (
    <Box
      key="header"
      flexDirection={narrow ? 'column' : 'row'}
      flexWrap="wrap"
      justifyContent="space-between"
      alignItems={narrow ? 'flex-start' : 'center'}
      gap={space(2)}
    >
      <Box gap={space(2)} flexWrap="wrap" alignItems="center">
        {m && m.project !== '' ? (
          <Text bold size={size.heading}>
            {m.project}
          </Text>
        ) : null}
        {m ? tag(ctx, 'neutral', m.run ?? t('noRun'), 'run') : null}
      </Box>
      <Box gap={space(2)} alignItems="center">
        {dim(ctx, stamp(ctx))}
        {button(ctx, 'refresh', t('refresh'), noop, { primary: true, hotkey: 'r' })}
      </Box>
    </Box>
  )
}

function errorLine(ctx: Ctx) {
  const { Text } = ctx.ui
  const d = ctx.data
  if (d.folder === 'no-engine') {
    return (
      <Text key="error" color={ink.bad}>
        {t('noEngine', { bf: d.engine })}
      </Text>
    )
  }
  if (!d.failed) return null
  return (
    <Text key="error" color={ink.bad}>
      {t('readFailed', { message: d.error })}
    </Text>
  )
}

function waitRow(ctx: Ctx, m: FactoryPanelModel, w: FactoryPanelWaiting) {
  const { Box, Text, Link } = ctx.ui
  const gate = w.kind === 'gate' ? m.gates.find(g => g.id === w.id) : undefined
  const text =
    w.kind === 'amendment'
      ? t('amendmentNames', { id: w.id, ids: w.names.join(', ') })
      : w.kind === 'gate'
        ? w.text || (gate ? t('gateRound', { kind: gate.kind, round: gate.round }) : w.id)
        : w.text || w.id
  return (
    <Box key={`wait-${w.id}`} flexDirection="column" gap={space(1)}>
      <Box gap={space(2)} flexWrap="wrap" alignItems="center">
        {tag(ctx, 'warn', t(WAIT_WORD[w.kind] ?? 'waitGate'))}
        <Text bold>{w.id}</Text>
      </Box>
      <Text>{text}</Text>
      {w.page && t('openLink') !== '' ? <Link href={w.page} label={t('openLink')} /> : null}
    </Box>
  )
}

function nowCard(ctx: Ctx, m: FactoryPanelModel, width?: number) {
  const { Box, Text, Svg } = ctx.ui
  const hasWait = m.waiting.length > 0
  const name: ToneName =
    m.standing === 'needs' || m.standing === 'blocked' || (m.standing === 'design' && hasWait)
      ? 'warn'
      : m.standing === 'finished'
        ? 'ok'
        : 'info'
  const rows: Kids = []
  rows.push(heading(ctx, t('now')))
  if (m.run === null) {
    rows.push(<Text key="stage" bold>{t('designStage')}</Text>)
    if (m.openGate === null && m.lastApprovedGate) {
      const g = m.lastApprovedGate
      rows.push(<Text key="last">{g.title || t('gateRound', { kind: g.kind, round: g.round })}</Text>)
    }
  } else if (m.finished) {
    rows.push(<Text key="done">{t('runFinished')}</Text>)
  } else {
    const k = m.openCheckpoint ? m.records[m.openCheckpoint] : undefined
    rows.push(
      <Box key="checkpoint" flexDirection="column">
        <Text bold>{m.openCheckpoint ? `${t('checkpoint')} ${m.openCheckpoint}` : t('noCheckpoint')}</Text>
        {k && k.title !== '' ? <Text color={ink.muted}>{k.title}</Text> : null}
      </Box>,
    )
  }
  if (m.run !== null) {
    const lines = m.segments.map(s => {
      const ids =
        s.words === 'landed' || s.sign === '✔' ? String(m.landed) : s.ids.join(', ')
      const round = s.round > 0 && !s.words.includes('round') ? ` ${t('roundWord', { round: s.round })}` : ''
      return `${s.sign} ${ids} ${s.words}${round}`
    })
    if (lines.length > 0) {
      rows.push(
        <Box key="units" flexDirection="column">
          {lines.map((l, i) => (
            <Text key={`unit-${i}`}>{l}</Text>
          ))}
        </Box>,
      )
    }
    if (m.units.length > 0) {
      const w = Math.max(10, Math.min(40, (width ?? ctx.cols) - 6))
      rows.push(
        <Box key="meter" flexDirection="column" gap={space(1)}>
          <Svg
            source={meterSource(m.landed, m.units.length, w, 8)}
            alt={t('sparkAlt', { landed: m.landed, total: m.units.length })}
            width={w}
            height={8}
          />
          {dim(ctx, t('unitsLanded', { landed: m.landed, total: m.units.length }))}
        </Box>,
      )
    }
  }
  const first = m.waiting[0]
  if (first) {
    rows.push(
      <Box key="waits" flexDirection="column" gap={space(2)}>
        <Text bold color={ink.warn}>
          {t('waitsOnYou')}
        </Text>
        {waitRow(ctx, m, first)}
        {m.waiting.length > 1 ? dim(ctx, t('unitsMore', { count: m.waiting.length - 1 })) : null}
      </Box>,
    )
  } else if (m.run !== null && !m.finished) {
    rows.push(dim(ctx, t('nothingWaits')))
  }
  return card(ctx, name, 'now', rows, width)
}

function pagesCard(ctx: Ctx, m: FactoryPanelModel, width?: number) {
  const { Box, Text, Link } = ctx.ui
  const open = ctx.choices.folds['pages'] === true
  const limit = 5
  const shown = open ? m.pages : m.pages.slice(0, limit)
  const rows: Kids = [heading(ctx, t('pages'))]
  if (m.pages.length === 0) rows.push(dim(ctx, t('pagesEmpty')))
  for (const p of shown) {
    rows.push(
      <Box key={`page-${p.name}`} gap={space(2)} flexWrap="wrap">
        <Link href={p.url} label={p.name} />
        {dim(ctx, p.at.slice(0, 10))}
      </Box>,
    )
  }
  if (m.pages.length > limit) {
    rows.push(button(ctx, 'fold:pages', open ? t('pagesFewer') : t('pagesAll'), noop))
  }
  void Text
  return card(ctx, null, 'pages', rows, width)
}

function stepCard(ctx: Ctx, s: FactoryPanelStep, width: number) {
  const { Box, Button, Text } = ctx.ui
  const gate = s.gate
  const state = gate ? gateState(gate.status) : null
  const count = s.id === 'build' ? t('checkpointsClosed', { closed: s.closed, total: s.total }) : t('records', { count: s.records })
  const label = stepName(s.id)
  return (
    <Box key={`step-${s.id}`} width={width} flexDirection="column">
      <Button key={`tab:${s.id}`} plain label={label} onPress={noop}>
        <Text bold>{label}</Text> <Text color={ink.muted}>{count}</Text>
        {state && gate ? ' ' : null}
        {state && gate ? tag(ctx, state.tone, state.word) : null}
        {state && gate ? ' ' : null}
        {state && gate ? <Text color={ink.muted}>{t('gateRoundShort', { round: gate.round })}</Text> : null}
      </Button>
    </Box>
  )
}

function chain(ctx: Ctx, m: FactoryPanelModel) {
  const { Box } = ctx.ui
  const cols = ctx.cols
  const narrow = widthOf(cols) === 'narrow'
  const per = narrow ? 1 : Math.max(2, Math.min(6, Math.floor(cols / 16)))
  const width = Math.max(10, Math.floor(cols / per) - 1)
  return (
    <Box key="chain" flexDirection="column" gap={space(2)}>
      {heading(ctx, t('chain'))}
      <Box flexDirection="row" flexWrap="wrap" gap={space(2)}>
        {m.steps.map(s => stepCard(ctx, s, width))}
      </Box>
    </Box>
  )
}

function placeholder(ctx: Ctx) {
  const { Box, Text } = ctx.ui
  const bar = (n: number) => '▒'.repeat(Math.max(4, Math.min(n, ctx.cols - 2)))
  return (
    <Box key="placeholder" flexDirection="column" gap={space(2)}>
      {heading(ctx, t('loading'))}
      <Text color={ink.muted}>{bar(24)}</Text>
      <Text color={ink.muted}>{bar(36)}</Text>
      <Text color={ink.muted}>{bar(18)}</Text>
    </Box>
  )
}

function foot(ctx: Ctx) {
  return dim(ctx, t('version', { version: VERSION }))
}

// Screen S-19: where the run stands, what waits on the human and the design chain.
export function overview(ctx: Ctx) {
  const { Box, Text } = ctx.ui
  const d = ctx.data
  const m = d.model
  const rows: Kids = []
  if (d.folder === 'not-factory') {
    rows.push(<Text key="not-factory">{t('notFactory')}</Text>)
    return <Box flexDirection="column" gap={space(3)}>{rows}</Box>
  }
  if (d.folder === 'no-engine') {
    rows.push(errorLine(ctx))
    return <Box flexDirection="column" gap={space(3)}>{rows}</Box>
  }
  rows.push(header(ctx, m))
  rows.push(errorLine(ctx))
  if (m === null) {
    if (d.phase === 'loading') rows.push(placeholder(ctx))
    rows.push(foot(ctx))
    return <Box flexDirection="column" gap={space(3)}>{rows}</Box>
  }
  if (m.progress !== '') {
    rows.push(<Text key="progress">{m.progress}</Text>)
  }
  for (const a of m.openAmendments) {
    rows.push(
      <Box key={`banner-${a.id}`} gap={space(2)} flexWrap="wrap" alignItems="center">
        {tag(ctx, 'warn', t('waitAmendment'))}
        <Text bold>{t('amendmentNames', { id: a.id, ids: a.ids.join(', ') })}</Text>
      </Box>,
    )
  }
  const wide = widthOf(ctx.cols) === 'wide'
  if (wide) {
    const nowW = Math.floor(ctx.cols * 0.6)
    const pagesW = ctx.cols - nowW - 1
    rows.push(
      <Box key="pair" flexDirection="row" gap={space(2)} alignItems="flex-start">
        {nowCard(ctx, m, nowW)}
        {pagesCard(ctx, m, pagesW)}
      </Box>,
    )
  } else {
    rows.push(nowCard(ctx, m))
    rows.push(pagesCard(ctx, m))
  }
  rows.push(chain(ctx, m))
  rows.push(foot(ctx))
  return (
    <Box flexDirection="column" gap={space(4)}>
      {rows}
    </Box>
  )
}
