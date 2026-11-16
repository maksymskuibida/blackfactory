// The one list of what the panel runs. Every command is an argument list for the engine,
// run with no shell. The panel runs nothing else, reads no file and writes none.

export const READS = {
  status: ['status', '--json'],
  progress: ['progress', '--json'],
  records: ['record', 'list', '--json'],
} as const

// The page kinds `bf view` builds. A view is read by kind and an optional record ID.
export const VIEW_KINDS = [
  'index',
  'gate',
  'questions',
  'ledger',
  'plan',
  'run',
  'checkpoint',
  'coverage',
] as const

export type ViewKind = (typeof VIEW_KINDS)[number]

const ID = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/

export function viewArgs(kind: ViewKind, id?: string): string[] {
  return id === undefined ? ['view', kind, '--json'] : ['view', kind, id, '--json']
}

// Whether an argument list (without the engine's name) is one of the reads above.
export function isRead(args: readonly string[]): boolean {
  for (const fixed of Object.values(READS)) {
    if (same(args, fixed)) return true
  }
  if (args[0] !== 'view') return false
  if (args[args.length - 1] !== '--json') return false
  const middle = args.slice(1, -1)
  if (middle.length < 1 || middle.length > 2) return false
  const kind = middle[0] as ViewKind
  if (!VIEW_KINDS.includes(kind)) return false
  const id = middle[1]
  return id === undefined || ID.test(id)
}

function same(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((x, i) => x === b[i])
}

// The engine's name is a bare command or a path: no spaces, quotes or shell characters.
export function isEngineName(bf: string): boolean {
  return /^[A-Za-z0-9_./~+-]{1,200}$/.test(bf)
}

export function argv(bf: string, args: readonly string[]): string[] {
  return [bf, ...args]
}
