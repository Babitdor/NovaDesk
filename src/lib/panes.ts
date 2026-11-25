/// The right panel's panes. The terminal is not one of them: it is a dock at the
/// bottom of the transcript, so it can stay open while a pane is read.
export const PANES = ['Changes', 'Files', 'Browser'] as const

export type Pane = (typeof PANES)[number]
