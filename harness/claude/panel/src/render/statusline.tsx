import type { FactoryPanelModel } from '../../types'
import { ink } from '../theme'
import { t } from '../strings'
import type { StringKey } from '../strings'
import type { ToneName } from '../tokens'
import { tone } from '../theme'
import { noop } from './context'
import type { Ctx, Kids } from './context'
import { tag } from './parts'

// The signs of the engine's progress line for units at work.
const WORK_SIGNS = ['▶', '◐', '↻', '●']
const STUCK_SIGN = '⚠'

const WAIT_WORD: Record<string, StringKey> = {
  gate: 'waitGate',
  question: 'waitQuestion',
  amendment: 'waitAmendment',
  parked: 'waitParked',
  blocked: 'waitBlocked',
}

const SIGN_OF: Record<string, string> = {
  design: '◌',
  idle: '○',
  building: '▶',
  needs: '●',
  blocked: STUCK_SIGN,
  verifying: '◎',
  finished: '■',
}

// The units at work in short form: signs with IDs and the round, three IDs at most, then +N.
export function workText(m: FactoryPanelModel): string {
  const entries: { sign: string; id: string; round: number }[] = []
  for (const s of m.segments) {
    if (!WORK_SIGNS.includes(s.sign)) continue
    for (const id of s.ids) entries.push({ sign: s.sign, id, round: s.round })
  }
  if (entries.length === 0) return ''
  const shown = entries.slice(0, 3)
  const groups: string[] = []
  let sign = ''
  for (const e of shown) {
    const label = e.round > 0 ? `${e.id} ${t('roundWord', { round: e.round })}` : e.id
    if (e.sign === sign) groups[groups.length - 1] += `, ${label}`
    else groups.push(`${e.sign} ${label}`)
    sign = e.sign
  }
  let out = groups.join(' ')
  if (entries.length > shown.length) out += ` ${t('unitsMore', { count: entries.length - shown.length })}`
  return out
}

function stateWord(m: FactoryPanelModel): string {
  const first = m.waiting[0]
  switch (m.standing) {
    case 'design':
      return t('standDesign')
    case 'needs':
      return first ? t(WAIT_WORD[first.kind] ?? 'waitGate') : ''
    case 'blocked': {
      // The engine's own word for the state of the first stuck unit.
      const u = m.units.find(x => x.state === 'parked' || x.state === 'blocked')
      return u ? u.state : ''
    }
    case 'verifying':
      return m.segments.find(s => s.words.startsWith('checkpoint'))?.words ?? ''
    case 'building':
      return m.segments.find(s => WORK_SIGNS.includes(s.sign))?.words ?? ''
    case 'finished':
      return t('standFinished')
    default:
      return t('standIdle')
  }
}

export function bandTone(m: FactoryPanelModel): ToneName {
  if (m.standing === 'needs' || m.standing === 'blocked') return 'warn'
  if (m.standing === 'finished') return 'ok'
  if (m.standing === 'design') return m.openGate ? 'warn' : 'info'
  return 'info'
}

function where(m: FactoryPanelModel): string {
  if (m.run === null) {
    const g = m.openGate ?? m.lastApprovedGate
    if (!g) return t('noRun')
    return g.title || t('gateRound', { kind: g.kind, round: g.round })
  }
  return m.openCheckpoint ? `${m.run} ${m.openCheckpoint}` : m.run
}

function nextWaits(m: FactoryPanelModel): string {
  const u = m.units.find(x => x.state === 'ready' || x.state === 'planned')
  return u ? t('nextWaits', { id: u.id, why: u.waitingFor ?? '' }).trim() : ''
}

// Screen S-23: one row, one button. Null when the data gives nothing to say yet.
export function statusRow(ctx: Ctx) {
  const { Button, Text } = ctx.ui
  const d = ctx.data
  if (d.folder === 'not-factory' || d.folder === 'no-engine') return null
  const m = d.model
  if (d.failed || (m === null && d.phase === 'ready')) {
    return (
      <Button key="open-panel" plain onPress={noop}>
        <Text color={ink.muted}>{t('readFailedBand')}</Text>
        <Text color={ink.muted}> </Text>
        <Text color={ink.accent}>{t('openPanel')}</Text>
      </Button>
    )
  }
  if (m === null) return null
  const name = bandTone(m)
  const c = tone(name)
  const parts: Kids = []
  parts.push(<Text key="sign" color={c.dot} bold>{`${SIGN_OF[m.standing] ?? ''} `}</Text>)
  parts.push(tag(ctx, name, stateWord(m), 'state'))
  const w = where(m)
  if (w !== '') parts.push(<Text key="where">{` ${w}`}</Text>)
  const work = workText(m)
  if (work !== '') parts.push(<Text key="work">{` ${work}`}</Text>)
  const stuck = m.units.filter(u => u.state === 'parked' || u.state === 'blocked').length
  if (stuck > 0) {
    parts.push(
      <Text key="stuck" color={ink.warn}>
        {` ${STUCK_SIGN} ${t('parkedCount', { count: stuck })}`}
      </Text>,
    )
  }
  const first = m.waiting[0]
  if (first && m.standing !== 'needs') {
    parts.push(<Text key="waits" color={ink.warn}>{` ${t(WAIT_WORD[first.kind] ?? 'waitGate')} ${first.id}`}</Text>)
  } else if (first) {
    parts.push(<Text key="waits" bold>{` ${first.id}`}</Text>)
  } else if (m.standing === 'idle') {
    const next = nextWaits(m)
    if (next !== '') parts.push(<Text key="next" color={ink.muted}>{` ${next}`}</Text>)
  }
  parts.push(<Text key="gap">{'  '}</Text>)
  parts.push(<Text key="open" color={ink.accent} bold>{t('openPanel')}</Text>)
  return (
    <Button key="open-panel" plain onPress={noop}>
      {parts}
    </Button>
  )
}
