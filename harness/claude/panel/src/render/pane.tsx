import { STEP_IDS } from '../model'
import { size, space } from '../theme'
import { t } from '../strings'
import { noop } from './context'
import type { Ctx } from './context'
import { button, dim } from './parts'
import { errorLine, overview, stepName } from './overview'
import { recordPage } from './record'
import { stageTab } from './tabs'

// The tab row: Overview and the stages of the design. Build opens from the overview.
function tabs(ctx: Ctx) {
  const { Box } = ctx.ui
  const current = ctx.choices.tab
  return (
    <Box key="tabs" flexDirection="row" flexWrap="wrap" gap={space(2)}>
      {button(ctx, 'tabs:overview', t('tabOverview'), noop, { primary: current === 'overview' })}
      {STEP_IDS.filter(id => id !== 'build').map(id => button(ctx, `tabs:${id}`, stepName(id), noop, { primary: current === id }))}
    </Box>
  )
}

// What the pane draws: the tab row and the screen of the chosen tab, or the record that is open.
export function pane(ctx: Ctx) {
  const { Box } = ctx.ui
  const { tab, open } = ctx.choices
  const model = ctx.data.model
  let body
  if (model === null || tab === 'overview') body = overview(ctx)
  else if (open !== null) body = recordPage(ctx, open)
  else body = stageTab(ctx, tab)
  const failed = model !== null && tab !== 'overview' ? errorLine(ctx) : null
  return (
    <Box flexDirection="column" gap={space(3)}>
      {ctx.data.folder === 'factory' ? tabs(ctx) : null}
      {failed}
      {body}
    </Box>
  )
}

export { dim, size }
