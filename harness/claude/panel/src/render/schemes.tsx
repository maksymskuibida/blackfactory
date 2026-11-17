import { BOX_LIMIT, componentPicture, componentScheme, planMap, planPicture } from '../layout'
import type { Picture } from '../layout'
import { STATE_GROUPS, ink, labelColors, size, space, stateColors } from '../theme'
import type { StateGroup } from '../theme'
import { t } from '../strings'
import type { StringKey } from '../strings'
import type { Ctx, Kids } from './context'
import { card, dim, heading, linkLine, swatch } from './parts'
import { idButton, planPage, readLine, stagePage } from './refs'

// Screen S-22: the component scheme and the plan map, each a picture in the pages' colours with a
// text form whose rows open the records, and the link to the full page.

const GROUP_WORD: Record<StateGroup, StringKey> = {
  planned: 'groupPlanned',
  building: 'groupBuilding',
  review: 'groupReview',
  fixing: 'groupFixing',
  landed: 'groupLanded',
  parked: 'groupParked',
}

function picture(ctx: Ctx, pic: Picture, alt: string, key: string) {
  if (pic === null) return null
  const { Svg } = ctx.ui
  // The picture is shown no wider than the room: its height follows.
  const width = Math.max(40, Math.min(pic.width, ctx.cols * 7))
  const height = Math.max(20, Math.round((pic.height * width) / pic.width))
  return <Svg key={key} source={pic.source} alt={alt} width={width} height={height} />
}

function textRow(ctx: Ctx, key: string, title: string, ids: string[], place: string, notes?: (id: string) => string) {
  const { Box, Text } = ctx.ui
  return (
    <Box key={key} flexDirection="row" flexWrap="wrap" gap={space(2)} alignItems="center">
      <Text bold color={ink.muted}>
        {title}
      </Text>
      {ids.map(id => [
        idButton(ctx, id, `${place}.${id}`),
        notes ? (
          <Text key={`note:${place}:${id}`} color={ink.muted}>
            {notes(id)}
          </Text>
        ) : null,
      ])}
    </Box>
  )
}

export function componentSchemeView(ctx: Ctx) {
  const { Box } = ctx.ui
  const m = ctx.data.model!
  const scheme = componentScheme(m)
  const count = scheme.layers.reduce((n, l) => n + l.ids.length, 0)
  const rows: Kids = [heading(ctx, t('schemeComponents'))]
  if (count === 0) {
    rows.push(dim(ctx, t('noComponents')))
    return card(ctx, null, 'scheme-components', rows)
  }
  for (const [from, to] of scheme.missing) {
    rows.push(
      <Box key={`missing:${from}:${to}`} flexDirection="row" flexWrap="wrap" gap={space(1)} alignItems="center">
        {idLineWarn(ctx, t('missingComponent', { from, to }), `missing.${from}.${to}`)}
      </Box>,
    )
  }
  if (scheme.edges.length === 0) rows.push(dim(ctx, t('noComponentLinks')))
  // The legend: the labels the boxes carry, in the pages' tones.
  const labels = Array.from(new Set(scheme.layers.flatMap(l => l.ids.map(id => m.records[id]!.label)).filter(x => x !== '')))
  const legend = labels.map(l => {
    const c = labelColors(l)
    return c ? swatch(ctx, c.bg, c.fg, l, `legend:${l}`) : null
  })
  if (legend.some(x => x !== null)) {
    rows.push(
      <Box key="legend" flexDirection="row" flexWrap="wrap" gap={space(2)} alignItems="center">
        {dim(ctx, t('legend'))}
        {legend}
      </Box>,
    )
  }
  const pic = count > BOX_LIMIT ? null : componentPicture(m, scheme)
  if (pic !== null) {
    rows.push(picture(ctx, pic, t('componentAlt', { count, layers: scheme.layers.length }), 'picture'))
  } else {
    rows.push(dim(ctx, t('tooLarge')))
  }
  rows.push(dim(ctx, t('schemeText')))
  for (const layer of scheme.layers) {
    rows.push(textRow(ctx, `layer:${layer.n}`, t('layerN', { n: layer.n }), layer.ids, `sc${layer.n}`))
  }
  const url = stagePage(m, 'architecture')
  rows.push(url !== null ? linkLine(ctx, url, t('stageLink')) : dim(ctx, t('notPublished')))
  return card(ctx, null, 'scheme-components', rows)
}

function idLineWarn(ctx: Ctx, text: string, key: string) {
  const { Text } = ctx.ui
  return (
    <Text key={key} color={ink.warn} bold>
      {text}
    </Text>
  )
}

export function planMapView(ctx: Ctx) {
  const { Box } = ctx.ui
  const m = ctx.data.model!
  const views = ctx.data.views
  const rows: Kids = [heading(ctx, t('schemePlan'))]
  const map = planMap(views)
  if (views.planState !== 'ready' && map.units === 0) {
    // Pending or failed: say so, never "no plan yet".
    rows.push(readLine(ctx, views.planState))
    return card(ctx, null, 'scheme-plan', rows)
  }
  if (map.units === 0) {
    rows.push(dim(ctx, t('noPlan')))
    return card(ctx, null, 'scheme-plan', rows)
  }
  // A refresh that failed keeps the last good map and says so above it.
  if (views.planState === 'failed') rows.push(readLine(ctx, views.planState))
  const legend = STATE_GROUPS.map(g => {
    const c = stateColors(g, 'light')
    return swatch(ctx, c.bg, c.fg, t(GROUP_WORD[g]), `legend:${g}`)
  })
  rows.push(
    <Box key="legend" flexDirection="row" flexWrap="wrap" gap={space(2)} alignItems="center">
      {dim(ctx, t('legend'))}
      {legend}
    </Box>,
  )
  const pic = map.units > BOX_LIMIT ? null : planPicture(m, map, { closed: t('checkpointClosed'), open: t('checkpointOpen'), stop: t('checkpointStop') })
  if (pic !== null) rows.push(picture(ctx, pic, t('planAlt', { count: map.units, waves: map.waves.length }), 'picture'))
  else rows.push(dim(ctx, t('tooLarge')))
  rows.push(dim(ctx, t('schemeText')))
  const stateOf = new Map(m.units.map(u => [u.id, u.state]))
  for (const wave of map.waves) {
    rows.push(textRow(ctx, `wave:${wave.n}`, t('waveN', { n: wave.n }), wave.ids, `pw${wave.n}`, id => stateOf.get(id) ?? ''))
  }
  const url = planPage(m)
  rows.push(url !== null ? linkLine(ctx, url, t('stageLink')) : dim(ctx, t('notPublished')))
  return card(ctx, null, 'scheme-plan', rows)
}

export { size }
