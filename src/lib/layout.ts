/// Sizes for the resizable chrome. The drag handles and the window-resize
/// clamp read the same limits, or a shrink can push a pane past its stop.
export const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value))

export const railMin = 180
export const railCollapsed = 34
export const panelMin = 240
export const panelLimit = 860
export const panelClearance = 390
export const dockMin = 96

export const railMax = () => Math.max(railMin, Math.min(360, window.innerWidth - panelMin - panelClearance))
export const panelMax = (sidebarWidth = railMin) =>
  clamp(window.innerWidth - sidebarWidth - panelClearance, panelMin, panelLimit)
export const dockMax = () => window.innerHeight * 0.6

export const defaultRailWidth = () => clamp(window.innerWidth * 0.18, railMin, railMax())
export const defaultPanelWidth = () => clamp(window.innerWidth * 0.39, panelMin, panelMax(defaultRailWidth()))
