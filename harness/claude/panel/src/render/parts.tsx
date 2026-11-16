import type { ToneName } from '../tokens'
import { ink, size, space, tone } from '../theme'
import { t } from '../strings'
import type { Ctx, Kids } from './context'

// A tag keeps the tokens' tone and always says its word: the tone never carries meaning alone.
export function tag(ctx: Ctx, name: ToneName, word: string, key?: string) {
  if (word === '') return null
  const { Text } = ctx.ui
  const c = tone(name)
  return (
    <Text key={key} color={c.fg} backgroundColor={c.bg} bold>
      {' '}
      {word}{' '}
    </Text>
  )
}

export function heading(ctx: Ctx, text: string) {
  if (text === '') return null
  const { Text } = ctx.ui
  return (
    <Text bold size={size.heading}>
      {text}
    </Text>
  )
}

export function dim(ctx: Ctx, text: string) {
  if (text === '') return null
  const { Text } = ctx.ui
  return (
    <Text color={ink.muted} size={size.small}>
      {text}
    </Text>
  )
}

// A card: spaced content in a rounded frame whose colour says the tone, beside its words.
export function card(ctx: Ctx, name: ToneName | null, key: string, children: Kids, width?: number) {
  const { Box } = ctx.ui
  const border = name === 'warn' ? ink.warn : name === 'ok' ? ink.good : name === 'crit' ? ink.bad : ink.muted
  return (
    <Box
      key={key}
      flexDirection="column"
      borderStyle="round"
      borderColor={border}
      padding={space(2)}
      gap={space(2)}
      width={width}
    >
      {children}
    </Box>
  )
}

export function button(
  ctx: Ctx,
  key: string,
  label: string,
  onPress: () => void,
  options?: { primary?: boolean; hotkey?: string; plain?: boolean },
) {
  if (label === '') return null
  const { Button } = ctx.ui
  return (
    <Button
      key={key}
      label={label}
      onPress={onPress}
      hotkey={options?.hotkey}
      variant={options?.primary ? 'primary' : 'secondary'}
      plain={options?.plain ? true : undefined}
    />
  )
}

// A link to a full page, drawn only for an https address.
export function linkLine(ctx: Ctx, url: string, label: string) {
  if (label === '' || !/^https:\/\/\S+$/.test(url)) return null
  const { Link } = ctx.ui
  return <Link key={`page:${label}`} href={url} label={label} />
}

// A swatch of a colour pair with its word: the legend of a picture.
export function swatch(ctx: Ctx, bg: string, fg: string, word: string, key: string) {
  if (word === '') return null
  const { Text } = ctx.ui
  return (
    <Text key={key} color={fg} backgroundColor={bg} bold>
      {' '}
      {word}{' '}
    </Text>
  )
}

// How long ago the panel read, in the panel's own words; empty before the first read.
export function stamp(ctx: Ctx): string {
  const at = ctx.data.readAt
  if (at <= 0) return t('stampNever')
  const seconds = Math.max(0, Math.round((ctx.now - at) / 1000))
  return seconds < 2 ? t('stampNow') : t('stamp', { seconds })
}
