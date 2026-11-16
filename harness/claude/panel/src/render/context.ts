import type { Elements, RenderChildren } from 'claude-code'
import type { FactoryPanelChoices, FactoryPanelData, FactoryPanelModel } from '../../types'

// The desktop surface's element table: the one surface the panel draws on.
export type Ui = Elements['desktop']

// The children of an element: elements, text and nothing.
export type Kids = RenderChildren[]

// What a screen draws from: the elements, the width, the clock and the panel's values.
export type Ctx = {
  ui: Ui
  // Columns of the box the screen draws into.
  cols: number
  // Milliseconds on the session clock, for the stamp.
  now: number
  data: FactoryPanelData
  choices: FactoryPanelChoices
}

export type Width = 'narrow' | 'medium' | 'wide'

// One column under 48 columns, the steps in one row that wraps from 48, two columns from 80.
export function widthOf(cols: number): Width {
  if (cols < 48) return 'narrow'
  if (cols < 80) return 'medium'
  return 'wide'
}

export function modelOf(ctx: Ctx): FactoryPanelModel | null {
  return ctx.data.model
}

// Presses are handled by the module's ui.press hook, by the button's key.
export const noop = (): void => {}
