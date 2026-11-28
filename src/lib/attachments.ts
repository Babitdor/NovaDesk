/// Picked files become prompt text: every harness already reads a path it is
/// given, so attaching is one line of the prompt and nothing in the protocol.
/// A path with whitespace is quoted so a shell-ish reader still sees one word.
export const attachmentPrompt = (draft: string, paths: string[]) => {
  if (!paths.length) return draft
  const list = paths.map((path) => /\s/.test(path) && !/^["'].*["']$/.test(path) ? `"${path}"` : path)
    .join(' ')
  const head = draft.trimEnd()
  return head ? head + ' ' + list : list
}
