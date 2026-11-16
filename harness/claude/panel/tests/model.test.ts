import { expect, test } from 'claude-code/testing'

import { amendment, designStage, everyState, finished, parked } from '../fixtures/factory'
import type { Scenario } from '../fixtures/factory'
import { buildModel, parseProgress } from '../src/model'

function model(s: Scenario) {
  return buildModel({ status: s.status, progress: { line: s.progress }, records: s.records, index: s.index })
}

test('a design stage with no run shows its open gate and no units', () => {
  const m = model(designStage())
  expect(m.run).toBe(null)
  expect(m.standing).toBe('design')
  expect(m.openGate?.id).toBe('ui-r1')
  expect(m.lastApprovedGate?.id).toBe('requirements-r1')
  expect(m.units.length).toBe(0)
  expect(m.waiting[0]?.kind).toBe('gate')
  expect(m.waiting[0]?.page).toBe('https://example.invalid/gate-ui-r1')
  expect(m.steps.map(s => s.id)).toEqual(['idea', 'requirements', 'ui', 'architecture', 'plan', 'build'])
  expect(m.steps[2]?.gate).toEqual({ round: 1, status: 'open' })
})

test('a run with units in every state keeps each state, round and what it waits for', () => {
  const m = model(everyState())
  const by = Object.fromEntries(m.units.map(u => [u.id, u]))
  expect(by['U-1']?.state).toBe('landed')
  expect(by['U-3']?.state).toBe('fixing')
  expect(by['U-3']?.round).toBe(1)
  expect(by['U-4']?.state).toBe('review')
  expect(by['U-4']?.round).toBe(1)
  expect(by['U-7']?.waitingFor).toBe('it depends on U-5, which has not landed')
  expect(by['U-6']?.waitingFor).toBe(null)
  expect(m.landed).toBe(2)
  expect(m.standing).toBe('building')
  expect(m.openCheckpoint).toBe('K-1')
  expect(m.segments.map(s => s.sign)).toEqual(['▶', '◐', '↻', '●', '✔'])
  expect(m.segments[1]).toEqual({ sign: '◐', ids: ['U-4'], words: 'in review', round: 1 })
  expect(m.segments[3]?.words).toBe('reviewed, ready to land')
  expect(m.progress.startsWith('▶ U-5 building')).toBe(true)
})

test('records carry their links both ways and each part its trace', () => {
  const m = model(everyState())
  expect(m.records['R-1']?.linkedBy).toContain('S-1')
  expect(m.records['S-1']?.links).toContain('R-1')
  expect(m.records['D-1']?.linkedBy).toContain('R-1')
  expect(m.records['U-1']?.links).toContain('R-1')
  expect(m.records['R-1']?.linkedBy).toContain('U-1')
  const tr = m.traces['R-1']!
  expect(tr.units).toEqual(['U-1', 'U-3', 'U-5', 'U-7'])
  expect(tr.checks).toContain('X-1')
  expect(tr.checkpoint).toBe('K-1')
  expect(m.traces['U-1']).toBe(undefined)
})

test('a parked unit and a blocked unit wait on the human and say why', () => {
  const m = model(parked())
  expect(m.standing).toBe('blocked')
  const kinds = m.waiting.map(w => `${w.kind}:${w.id}`)
  expect(kinds).toEqual(['parked:U-3', 'blocked:U-4'])
  expect(m.waiting[0]?.text).toBe('the builder asked for a decision')
})

test('an open amendment names its records and puts the run in needs', () => {
  const m = model(amendment())
  expect(m.openAmendments.map(a => a.id)).toEqual(['A-2'])
  expect(m.openAmendments[0]?.ids).toEqual(['R-2', 'S-1'])
  expect(m.standing).toBe('needs')
  expect(m.waiting.find(w => w.kind === 'amendment')?.names).toEqual(['R-2', 'S-1'])
})

test('a finished run has every checkpoint closed', () => {
  const m = model(finished())
  expect(m.finished).toBe(true)
  expect(m.standing).toBe('finished')
  expect(m.openCheckpoint).toBe(null)
  const build = m.steps[5]!
  expect(build.closed).toBe(2)
  expect(build.total).toBe(2)
})

test('pages come newest first by name', () => {
  const m = model(everyState())
  expect(m.pages.map(p => p.name)).toEqual(['gate-plan-r1', 'plan', 'questions'])
})

test('the progress line is split into signs, IDs, words and rounds', () => {
  const segs = parseProgress('▶ U-1 (round 2), U-2 building · ■ K-1, K-2 checkpoint closed')
  expect(segs[0]).toEqual({ sign: '▶', ids: ['U-1', 'U-2'], words: 'building', round: 2 })
  expect(segs[1]?.ids).toEqual(['K-1', 'K-2'])
  expect(parseProgress('no unit has started')).toEqual([])
})

test('a read with nothing in it builds an empty model', () => {
  const m = buildModel({ status: null, progress: null, records: null })
  expect(m.units).toEqual([])
  expect(m.project).toBe('')
  expect(m.standing).toBe('design')
})
