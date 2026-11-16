// Invented factories for the tests: what the engine's read commands print for a design stage with no
// run, a run with units in every state, a parked unit, an open amendment and a finished run, and a
// large one. Each scenario is plain data; `answer` maps an argument list to what the engine prints.

export type Scenario = {
  status: Record<string, unknown>
  progress: string
  records: Record<string, unknown>[]
  index: Record<string, unknown>
}

type Rec = Record<string, unknown>

function designRecords(): Rec[] {
  return [
    { id: 'I-1', kind: 'idea', title: 'A small tool that keeps a list', text: 'Idea text.' },
    { id: 'R-1', kind: 'requirement', label: 'critical', links: ['D-1'], title: 'The tool keeps a list.', acceptance: ['It lists.'] },
    { id: 'R-2', kind: 'requirement', label: 'high', links: ['D-1'], title: 'The list survives a restart.', acceptance: ['It stays.'] },
    { id: 'S-1', kind: 'screen', label: 'high', links: ['J-1', 'R-1'], title: 'The list screen.', text: 'Interface: page.' },
    { id: 'J-1', kind: 'journey', label: 'high', links: ['R-1'], title: 'The person adds an item.', text: '1. Add.' },
    { id: 'C-1', kind: 'component', label: 'core', links: ['R-1'], title: 'The store holds the list.', text: 'A table.' },
    { id: 'D-1', kind: 'decision', title: 'Keep the list in a file.', chosen: 'A file.', rejected: 'A server.' },
    { id: 'T-1', kind: 'target', links: ['C-1'], title: 'Adding is quick.', measure: 'Under 50 ms.' },
    { id: 'TH-1', kind: 'threat', label: 'high', links: ['C-1'], title: 'The file is lost.', mitigation: 'A copy.' },
  ]
}

function planRecords(unitCount: number): Rec[] {
  const out: Rec[] = []
  for (let i = 1; i <= unitCount; i++) {
    const k = i <= Math.ceil(unitCount / 2) ? 'K-1' : 'K-2'
    out.push({
      id: `U-${i}`,
      kind: 'unit',
      title: `Unit ${i} builds a part.`,
      text: 'Text.',
      checkpoint: k,
      delivers: i % 2 === 1 ? ['R-1', 'C-1'] : ['R-2'],
      files: ['src/'],
      estimate: 1,
    })
    out.push({
      id: `X-${i}`,
      kind: 'check',
      title: `The tests of unit U-${i} pass.`,
      unit: `U-${i}`,
      how: 'command',
      command: 'cargo test',
      scope: 'unit',
      proves: i % 2 === 1 ? ['R-1', 'C-1'] : ['R-2'],
    })
  }
  const half = Math.ceil(unitCount / 2)
  out.push({
    id: 'K-1',
    kind: 'checkpoint',
    title: 'The first half works.',
    after: Array.from({ length: half }, (_, i) => `U-${i + 1}`),
    checks: ['X-1'],
    proves: ['R-1', 'C-1'],
  })
  out.push({
    id: 'K-2',
    kind: 'checkpoint',
    title: 'The second half works.',
    after: Array.from({ length: unitCount - half }, (_, i) => `U-${half + i + 1}`),
    checks: ['X-2'],
    proves: ['R-2'],
  })
  return out
}

function gate(kind: string, round: number, status: string): Rec {
  return {
    id: `${kind}-r${round}`,
    kind,
    round,
    status,
    ref: null,
    opened_at: '2026-01-01T10:00:00Z',
    simulated: false,
    human_turn: round,
    note: '',
    quote: '',
    human_message: '',
    caution: '',
    transcript_check: '',
  }
}

function page(name: string, at: string): Rec {
  return { name, at, url: `https://example.invalid/${name}` }
}

function unit(id: string, state: string, waiting: string | null = null, rounds = 0): Rec {
  return {
    id,
    state,
    waiting_for: waiting,
    review_rounds: Array.from({ length: rounds }, () => ({})),
    scope_grants: [],
    scope_request: null,
    checkpoint_fixes: [],
  }
}

function base(): Scenario {
  return {
    status: {
      project: 'tinylist',
      run: null,
      events: 40,
      units: [],
      gates: [gate('idea', 1, 'approved'), gate('requirements', 1, 'approved'), gate('ui', 1, 'open')],
      amendments: [],
      checkpoints_closed: [],
      checkpoint_fix: { done: [], open: null },
      pages: [page('gate-idea-r1', '2026-01-01T10:05:00Z'), page('gate-requirements-r1', '2026-01-01T11:05:00Z'), page('gate-ui-r1', '2026-01-01T12:05:00Z')],
      unit_counts: {},
    },
    progress: 'no unit has started',
    records: designRecords(),
    index: {
      sections: [
        {
          id: 'gates',
          rows: [
            { id: 'idea-r1', title: 'The idea gate, round 1' },
            { id: 'requirements-r1', title: 'The requirements gate, round 1' },
            { id: 'ui-r1', title: 'The ui gate, round 1' },
          ],
        },
      ],
    },
  }
}

// A design stage with no run: the ui gate waits.
export function designStage(): Scenario {
  return base()
}

// A run with units in every state: landed, reviewed, fixing, in review, building, ready, planned.
export function everyState(): Scenario {
  const s = base()
  s.records = [...designRecords(), ...planRecords(8)]
  s.status = {
    ...s.status,
    run: 'run-1',
    events: 120,
    gates: ['idea', 'requirements', 'ui', 'architecture', 'plan'].map(k => gate(k, 1, 'approved')),
    units: [
      unit('U-1', 'landed'),
      unit('U-2', 'reviewed'),
      unit('U-3', 'fixing', null, 1),
      unit('U-4', 'review'),
      unit('U-5', 'running'),
      unit('U-6', 'ready'),
      unit('U-7', 'planned', 'it depends on U-5, which has not landed'),
      unit('U-8', 'landed'),
    ],
    unit_counts: { landed: 2, reviewed: 1, fixing: 1, review: 1, running: 1, ready: 1, planned: 1 },
    pages: [page('gate-plan-r1', '2026-01-02T10:00:00Z'), page('plan', '2026-01-02T09:00:00Z'), page('questions', '2026-01-01T09:00:00Z')],
  }
  s.progress =
    '▶ U-5 building · ◐ U-4 in review, round 1 · ↻ U-3 fixing, round 1 · ● U-2 reviewed, ready to land · ✔ U-1, U-8 landed'
  return s
}

// A run where one unit is parked and one is blocked.
export function parked(): Scenario {
  const s = everyState()
  s.status = {
    ...s.status,
    units: [
      unit('U-1', 'landed'),
      unit('U-2', 'landed'),
      unit('U-3', 'parked', 'the builder asked for a decision'),
      unit('U-4', 'blocked', 'two reviews found the same fault'),
      unit('U-5', 'running'),
      unit('U-6', 'ready'),
      unit('U-7', 'planned', 'it depends on U-5, which has not landed'),
      unit('U-8', 'landed'),
    ],
  }
  s.progress = '▶ U-5 building · ✔ U-1, U-2, U-8 landed · ⚠ U-3 parked, U-4 blocked'
  return s
}

// A run with one open amendment that names two records.
export function amendment(): Scenario {
  const s = everyState()
  s.status = {
    ...s.status,
    amendments: [
      { id: 'A-1', ids: ['R-1'], status: 'approved', withdrawn_why: '' },
      { id: 'A-2', ids: ['R-2', 'S-1'], status: 'open', withdrawn_why: '' },
    ],
  }
  return s
}

// A run with nothing building: the next unit waits for another.
export function idle(): Scenario {
  const s = everyState()
  s.status = {
    ...s.status,
    units: [
      unit('U-1', 'landed'),
      unit('U-2', 'planned', 'it depends on U-3, which has not landed'),
      unit('U-3', 'planned', 'its checkpoint K-1 builds on K-0, which is not closed'),
    ],
  }
  s.progress = '✔ U-1 landed'
  return s
}

// A checkpoint whose units all landed and which is being verified.
export function verifying(): Scenario {
  const s = everyState()
  s.records = [...designRecords(), ...planRecords(2)]
  s.status = {
    ...s.status,
    units: [unit('U-1', 'landed'), unit('U-2', 'landed')],
    checkpoints_closed: [],
  }
  s.progress = '✔ U-1, U-2 landed · ◎ K-1 checkpoint being verified'
  return s
}

// A finished run: every checkpoint is closed.
export function finished(): Scenario {
  const s = everyState()
  s.records = [...designRecords(), ...planRecords(4)]
  s.status = {
    ...s.status,
    units: [unit('U-1', 'landed'), unit('U-2', 'landed'), unit('U-3', 'landed'), unit('U-4', 'landed')],
    checkpoints_closed: ['K-1', 'K-2'],
  }
  s.progress = '✔ U-1, U-2, U-3, U-4 landed · ■ K-1, K-2 checkpoint closed'
  return s
}

// A large factory: 60 units, 30 components and 500 records in all.
export function large(): Scenario {
  const s = everyState()
  const records: Rec[] = [...designRecords(), ...planRecords(60)]
  for (let i = 2; i <= 30; i++) {
    records.push({ id: `C-${i}`, kind: 'component', label: 'main', links: ['R-1', 'R-2'], title: `Component ${i}.`, text: 'Text.' })
  }
  let n = 100
  while (records.length < 500) {
    n++
    records.push({ id: `R-${n}`, kind: 'requirement', label: 'low', links: ['C-1', `C-${(n % 29) + 2}`], title: `Requirement ${n}.`, acceptance: ['Holds.'] })
  }
  s.records = records
  s.status = {
    ...s.status,
    units: Array.from({ length: 60 }, (_, i) =>
      unit(`U-${i + 1}`, i < 30 ? 'landed' : i < 34 ? 'running' : i < 36 ? 'ready' : 'planned', i >= 36 ? 'it depends on U-34, which has not landed' : null),
    ),
  }
  s.progress = '▶ U-31, U-32, U-33, U-34 building · ✔ U-1, U-2 landed'
  return s
}

export type Answer = { exitCode: number; stdout: string; stderr: string }

// What the engine prints for an argument list (the engine's name first), or a refusal for any other.
export function answer(s: Scenario, argv: readonly string[]): Answer {
  const args = argv.slice(1).join(' ')
  const ok = (v: unknown): Answer => ({ exitCode: 0, stdout: JSON.stringify(v), stderr: '' })
  if (args === 'status --json') return ok(s.status)
  if (args === 'progress --json') return ok({ line: s.progress })
  if (args === 'record list --json') return ok(s.records)
  if (args === 'view index --json') return ok(s.index)
  return { exitCode: 1, stdout: '', stderr: `unexpected: ${args}` }
}

export const NOT_A_FACTORY_ANSWER: Answer = {
  exitCode: 2,
  stdout: JSON.stringify({
    ok: false,
    refused: 'use a Black Factory project here',
    because: 'no `factory/config.toml` was found in this folder or any folder above it',
    do_this: ['Change into the project folder.'],
  }),
  stderr: '',
}
