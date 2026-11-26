/// The right panel's panes. The terminal is not one of them: it is a dock at the
/// bottom of the transcript, so it can stay open while a pane is read.
///
/// Chat is not one either: it is the transcript, which owns the whole column
/// whenever no pane is open, so "the panel is closed" and "the view is Chat" are
/// one state rather than two flags that can disagree.
export const PANES = ['Files', 'GitHub', 'Board', 'Preview'] as const

export type Pane = (typeof PANES)[number]

/// Everything the switcher can hold: the panes, plus the transcript they open
/// beside.
export const VIEWS = ['Chat', ...PANES] as const

export type View = (typeof VIEWS)[number]
