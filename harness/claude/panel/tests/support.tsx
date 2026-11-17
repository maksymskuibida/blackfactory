import { mock } from 'claude-code/testing'

import { NOT_A_FACTORY_ANSWER, answer } from '../fixtures/factory'
import type { Scenario } from '../fixtures/factory'

// The pieces every engine-driven test shares: the engine beneath the plugin, answered from memory.

export const PANE = {
  plugin: 'factory-panel',
  component: 'Pane',
  requestId: 'factory-panel',
} as const

export const BAND_FOR_TESTS = {
  plugin: 'factory-panel',
  component: 'AbovePrompt',
  requestId: 'band',
} as const

export function paneProps(columns: number) {
  return {
    title: 'Overview',
    isFocused: false,
    bodyColumns: columns,
    placement: 'dock',
    scroll: { offset: 0, bodyRows: 40 },
    view: {},
  } as const
}

export function bandProps(over: { hasSurvey?: boolean; isCollapsed?: boolean; columns?: number } = {}) {
  return {
    hasSurvey: over.hasSurvey ?? false,
    isCollapsed: over.isCollapsed ?? false,
    isWorking: false,
    maxRows: 6,
    bodyColumns: over.columns ?? 100,
    scroll: { offset: 0, bodyRows: 6 },
    view: {},
  } as const
}

export type World = {
  scenario: Scenario | 'not-factory' | 'no-engine'
  calls: string[]
  panes: { id: string; isPlaced: boolean }[]
  opened: number
  placed: boolean
  // When set, every read fails with this message.
  failWith: string | null
}

// Stands for the engine beneath the plugin. `on` is the test's own registration of hooks.
export function engine(on: any, scenario: World['scenario'], start = 1_000_000) {
  const world: World = { scenario, calls: [], panes: [], opened: 0, placed: true, failWith: null }
  const clock = mock.clock(on, { now: start })
  on('command.register', () => ({ value: { command: 'factory-panel' } }))
  on('session.start', (_: unknown, e: { cwd: string }) => ({ cwd: e.cwd }))
  on('session.attach', (_: unknown, e: { clientId: string }) => ({ clientId: e.clientId }))
  on('session.surfaces', () => ({ value: ['desktop'] }))
  on('turn.complete', (_: unknown, e: unknown) => e)
  on('ui.open', (_: unknown, e: { id: string }) => {
    world.opened += 1
    if (world.placed && !world.panes.some(p => p.id === e.id)) world.panes.push({ id: e.id, isPlaced: true })
    return { value: world.placed ? { isPlaced: true } : { isPlaced: false, reason: 'no room' } }
  })
  on('ui.panes', () => ({ value: world.panes }))
  on('ui.close', (_: unknown, e: { id: string }) => {
    world.panes = world.panes.filter(p => p.id !== e.id)
  })
  on('process.run', (_: unknown, e: { argv: readonly string[] }) => {
    world.calls.push(e.argv.slice(1).join(' '))
    if (world.failWith !== null) {
      return { value: { exitCode: 1, stdout: '', stderr: world.failWith, isStdoutTruncated: false, isStderrTruncated: false } }
    }
    if (world.scenario === 'no-engine') throw new Error('spawn bf ENOENT')
    if (world.scenario === 'not-factory') return { value: { ...NOT_A_FACTORY_ANSWER, isStdoutTruncated: false, isStderrTruncated: false } }
    return { value: { ...answer(world.scenario, e.argv), isStdoutTruncated: false, isStderrTruncated: false } }
  })
  // What the engine draws itself where the plugin leaves a site.
  on('ui.render', ($: any, e: any) => {
    const { Text } = $.ui.resolve(e)
    return <Text>engine</Text>
  })
  return { world, clock }
}

export function statusCalls(world: World): number {
  return world.calls.filter(c => c === 'status --json').length
}
