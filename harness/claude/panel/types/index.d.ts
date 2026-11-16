// The panel's contract: the model its reads are joined into, the session values of the person's
// choices, the strings function and the tokens module. The screens of later units draw from these
// and add no read of their own: src/reads.ts is the whole list of commands.

export type FactoryPanelTone = 'ok' | 'info' | 'warn' | 'crit' | 'neutral'

// One record as `bf record list` prints it, with its links both ways.
export type FactoryPanelRecord = {
  id: string
  kind: string
  title: string
  text: string
  label: string
  // Every field the engine printed, as printed.
  fields: Record<string, unknown>
  // The records this one names (links, delivers, proves, after, checks, checkpoint, unit and the like).
  links: string[]
  // The records that name this one.
  linkedBy: string[]
}

export type FactoryPanelUnitState =
  | 'planned'
  | 'ready'
  | 'running'
  | 'review'
  | 'fixing'
  | 'reviewed'
  | 'landed'
  | 'parked'
  | 'blocked'

export type FactoryPanelUnit = {
  id: string
  title: string
  state: FactoryPanelUnitState | string
  // What the unit waits for, in the engine's words; null when it waits for nothing.
  waitingFor: string | null
  // The review round shown with the state: the one under review, or the one being fixed; 0 otherwise.
  round: number
  checkpoint: string
  delivers: string[]
  checks: string[]
}

export type FactoryPanelCheckpointPhase = 'planned' | 'building' | 'verifying' | 'fixing' | 'closed'

export type FactoryPanelCheckpoint = {
  id: string
  title: string
  units: string[]
  closed: boolean
  phase: FactoryPanelCheckpointPhase
}

export type FactoryPanelGateStatus = 'draft' | 'open' | 'approved' | 'rejected'

export type FactoryPanelGate = {
  id: string
  kind: string
  round: number
  status: FactoryPanelGateStatus | string
  // The address of the gate's published page, when it has one.
  page: string | null
  // The engine's title for it (from the index page), when it printed one.
  title: string
}

export type FactoryPanelAmendment = {
  id: string
  ids: string[]
  status: 'open' | 'approved' | 'withdrawn' | string
}

export type FactoryPanelPage = { name: string; url: string; at: string }

// What proves and delivers one designed part.
export type FactoryPanelTrace = {
  part: string
  units: string[]
  checks: string[]
  checkpoint: string | null
}

export type FactoryPanelStepId = 'idea' | 'requirements' | 'ui' | 'architecture' | 'plan' | 'build'

export type FactoryPanelStep = {
  id: FactoryPanelStepId
  records: number
  // The latest gate of the step and its round; null for a step with no gate yet (Build has none).
  gate: { round: number; status: string } | null
  // For Build: checkpoints closed of all.
  closed: number
  total: number
}

export type FactoryPanelWaitKind = 'gate' | 'question' | 'amendment' | 'parked' | 'blocked'

export type FactoryPanelWaiting = {
  kind: FactoryPanelWaitKind
  id: string
  // The engine's words for it: a title, or what the unit waits for.
  text: string
  // The IDs an amendment names.
  names: string[]
  // The address of its page, when there is one.
  page: string | null
}

// One segment of the engine's progress line: a sign, the IDs, the words and the round.
export type FactoryPanelSegment = { sign: string; ids: string[]; words: string; round: number }

// Where the run stands, the one word the status line and the Now card read.
export type FactoryPanelStanding =
  | 'design'
  | 'idle'
  | 'building'
  | 'needs'
  | 'blocked'
  | 'verifying'
  | 'finished'

export type FactoryPanelModel = {
  project: string
  run: string | null
  standing: FactoryPanelStanding
  events: number
  // The progress line exactly as the engine prints it.
  progress: string
  segments: FactoryPanelSegment[]
  records: Record<string, FactoryPanelRecord>
  order: string[]
  units: FactoryPanelUnit[]
  unitCounts: Record<string, number>
  landed: number
  checkpoints: FactoryPanelCheckpoint[]
  openCheckpoint: string | null
  finished: boolean
  traces: Record<string, FactoryPanelTrace>
  gates: FactoryPanelGate[]
  openGate: FactoryPanelGate | null
  lastApprovedGate: FactoryPanelGate | null
  amendments: FactoryPanelAmendment[]
  openAmendments: FactoryPanelAmendment[]
  // Newest first.
  pages: FactoryPanelPage[]
  steps: FactoryPanelStep[]
  waiting: FactoryPanelWaiting[]
  // The engine's titles by row ID, from its index page.
  titles: Record<string, string>
}

export type FactoryPanelTab = 'overview' | FactoryPanelStepId

// The person's choices in the panel, kept for the session and nowhere else.
export type FactoryPanelChoices = {
  tab: FactoryPanelTab
  // The record open on a record page, and the records the person came from (the way back).
  open: string | null
  back: string[]
  folds: Record<string, boolean>
  search: string
}

export type FactoryPanelFolder = 'unknown' | 'factory' | 'not-factory' | 'no-engine'

// What the last reads left: the model, when it was read and the engine's message when a read failed.
export type FactoryPanelData = {
  phase: 'loading' | 'ready'
  folder: FactoryPanelFolder
  model: FactoryPanelModel | null
  // The engine command the panel ran when it could not be run (the bf option).
  engine: string
  error: string
  // True while the last read failed; the model above is then the last good one.
  failed: boolean
  // Milliseconds on the session clock; 0 before the first read.
  readAt: number
  events: number
}

// What the hooks know about the session.
export type FactoryPanelSession = {
  isDesktop: boolean
  // The person closed the pane: it stays closed for the session.
  isClosed: boolean
  // The pane was opened once by the panel itself.
  wasOpened: boolean
  // The pane is placed beside the chat now: the status line yields to it.
  isShown: boolean
}

export type FactoryPanelStringKey = string

// The strings function: the panel's own words by key. An empty string hides its element.
export type FactoryPanelStrings = (key: FactoryPanelStringKey, values?: Record<string, string | number>) => string

export type FactoryPanelToneColors = { bg: string; fg: string; dot: string }

// The tokens module (src/tokens.ts), written by views/dev/build.py from views/tokens/tokens.json.
export type FactoryPanelTokens = {
  version: number
  light: Record<string, string>
  dark: Record<string, string>
  type: Record<string, number>
  space: Record<string, number>
  radius: Record<string, number>
  fonts: { body: string; mono: string }
  tones: Record<string, { light: FactoryPanelToneColors; dark: FactoryPanelToneColors }>
}

declare module 'claude-code' {
  interface PluginState {
    'factory-panel': {
      choices: Shaped<FactoryPanelChoices>
      data: Shaped<FactoryPanelData>
      session: Shaped<FactoryPanelSession>
    }
  }
}
