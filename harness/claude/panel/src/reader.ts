import { argv, isEngineName, isRead } from './reads'

// The reader takes one port: how to run an argument list. Nothing else touches the host.
export type RunResult = { exitCode: number; stdout: string; stderr: string }
export type RunPort = (argv: string[]) => Promise<RunResult>

export type ReadFailure = {
  ok: false
  // not-factory: the folder holds no factory; no-engine: the command could not be run;
  // failed: the engine answered with a refusal or something that is not JSON.
  reason: 'not-factory' | 'no-engine' | 'failed'
  message: string
}
export type ReadSuccess = { ok: true; json: unknown }
export type ReadResult = ReadSuccess | ReadFailure

// What the engine says when the folder holds no factory (its own refusal).
export const NOT_A_FACTORY = 'use a Black Factory project here'

export class ReadRefused extends Error {
  constructor(args: readonly string[]) {
    super(`not a read of the panel: ${args.join(' ')}`)
  }
}

export function createReader(run: RunPort, bf: string) {
  async function read(args: readonly string[]): Promise<ReadResult> {
    if (!isRead(args)) throw new ReadRefused(args)
    if (!isEngineName(bf)) {
      return { ok: false, reason: 'no-engine', message: bf }
    }
    let out: RunResult
    try {
      out = await run(argv(bf, args))
    } catch (err) {
      // A command that started and then ran out of time is a failed read, not a missing engine.
      const message = errorText(err)
      const reason = /time(d)?[ -]?out/i.test(message) ? 'failed' : 'no-engine'
      return { ok: false, reason, message }
    }
    return classify(out)
  }
  return { read }
}

export type Reader = ReturnType<typeof createReader>

export function classify(out: RunResult): ReadResult {
  let json: unknown
  let parsed = false
  try {
    json = JSON.parse(out.stdout)
    parsed = true
  } catch {
    parsed = false
  }
  if (out.exitCode === 0 && parsed) return { ok: true, json }
  if (parsed && json && typeof json === 'object') {
    const refusal = json as Record<string, unknown>
    if (refusal.refused === NOT_A_FACTORY) {
      return { ok: false, reason: 'not-factory', message: text(refusal.because) }
    }
    const because = text(refusal.because)
    const refused = text(refusal.refused)
    return { ok: false, reason: 'failed', message: because || refused || out.stderr.trim() }
  }
  const message = out.stderr.trim() || out.stdout.trim()
  return { ok: false, reason: 'failed', message: message.slice(0, 400) }
}

function text(v: unknown): string {
  return typeof v === 'string' ? v : ''
}

function errorText(err: unknown): string {
  if (err instanceof Error) return err.message
  return String(err)
}
