import { STEP_IDS, STEP_KINDS } from '../model'
import { ink, space } from '../theme'
import { t } from '../strings'
import { noop } from './context'
import type { Ctx, Kids } from './context'
import { button, dim, heading } from './parts'
import { overview, stepName } from './overview'

// The tab row: Overview and the six steps of the design chain.
function tabs(ctx: Ctx) {
  const { Box } = ctx.ui
  const current = ctx.choices.tab
  return (
    <Box key="tabs" flexDirection="row" flexWrap="wrap" gap={space(2)}>
      {button(ctx, 'tabs:overview', t('tabOverview'), noop, { primary: current === 'overview' })}
      {STEP_IDS.map(id => button(ctx, `tabs:${id}`, stepName(id), noop, { primary: current === id }))}
    </Box>
  )
}

// A step's own tab: its records as the engine titled them. Later units draw the record pages.
function stepTab(ctx: Ctx, id: (typeof STEP_IDS)[number]) {
  const { Box, Text } = ctx.ui
  const m = ctx.data.model
  const rows: Kids = [heading(ctx, stepName(id))]
  if (m === null) return <Box flexDirection="column" gap={space(2)}>{rows}</Box>
  if (id === 'build') {
    if (m.checkpoints.length === 0) rows.push(dim(ctx, t('tabEmpty')))
    for (const k of m.checkpoints) {
      rows.push(
        <Text key={k.id}>
          {k.id} {k.title}
        </Text>,
      )
    }
  } else {
    const kinds = STEP_KINDS[id]
    const ids = m.order.filter(r => kinds.includes(m.records[r]!.kind))
    if (ids.length === 0) rows.push(dim(ctx, t('tabEmpty')))
    for (const r of ids.slice(0, 200)) {
      rows.push(
        <Text key={r}>
          <Text bold>{r}</Text> <Text color={ink.muted}>{m.records[r]!.title}</Text>
        </Text>,
      )
    }
  }
  return (
    <Box flexDirection="column" gap={space(1)}>
      {rows}
    </Box>
  )
}

// What the pane draws: the tab row and the screen of the chosen tab.
export function pane(ctx: Ctx) {
  const { Box } = ctx.ui
  const tab = ctx.choices.tab
  const body = tab === 'overview' || ctx.data.model === null ? overview(ctx) : stepTab(ctx, tab)
  return (
    <Box flexDirection="column" gap={space(3)}>
      {ctx.data.folder === 'factory' ? tabs(ctx) : null}
      {body}
    </Box>
  )
}
