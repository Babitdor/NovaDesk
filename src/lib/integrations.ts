/// The two numbers and two lists the Tools & integrations sheet shows, worked
/// out from answers the app already has.
///
/// No Tauri import and no import at all lives in this module: the sheet calls
/// `invoke` itself, and staying import-free is what lets `node --test` exercise
/// the counting and the row building.

/// The part of a voice or speech catalogue entry the Apps list needs. Both
/// catalogues carry these fields; anything else is the sheet's business.
export type CatalogEntry = { id: string; name: string; bytes: number }

/// One downloaded thing, as the Apps section lists it.
export type AppRow = { id: string; name: string; detail: string }

/// MB and GB are the only sizes a model download is measured in at this scale.
/// shortcut: no bytes/KB case; src/lib/voice.ts's formatBytes covers the rest of
/// the app, and importing it would make this module unloadable by node --test.
function size(bytes: number): string {
  return bytes >= 1024 ** 3
    ? (bytes / 1024 ** 3).toFixed(1) + ' GB'
    : Math.max(1, Math.round(bytes / 1024 ** 2)) + ' MB'
}

/// What Nova has actually downloaded: the dictation models and speech voices
/// present on disk, plus the speech runtime when it is installed.
///
/// Only installed entries appear. The catalogues are long and mostly *available*
/// downloads, and a section that listed every one of them would say less about
/// this machine than the two lines that are true. An empty list is the honest
/// answer for a fresh install, and the section says where to get some.
export function appRows(
  voice: CatalogEntry[],
  installedVoice: string[],
  speak: CatalogEntry[],
  installedSpeak: string[],
  runtimeReady: boolean,
): AppRow[] {
  const rows: AppRow[] = []
  for (const model of voice) {
    if (installedVoice.includes(model.id)) {
      rows.push({ id: 'voice:' + model.id, name: model.name, detail: 'Dictation model · ' + size(model.bytes) })
    }
  }
  for (const voiceEntry of speak) {
    if (installedSpeak.includes(voiceEntry.id)) {
      rows.push({ id: 'speak:' + voiceEntry.id, name: voiceEntry.name, detail: 'Speech voice · ' + size(voiceEntry.bytes) })
    }
  }
  if (runtimeReady) {
    rows.push({ id: 'speech-runtime', name: 'Speech runtime', detail: 'Piper · installed with the first voice' })
  }
  return rows
}

/// The number beside each section heading.
///
/// Skills are counted by what is actually handed over: a switched-off skill is
/// still listed in the section, so the heading shows the enabled count and the
/// `disabled` counter is what the sheet uses to say how many are off.
export type SectionCounts = {
  tools: number
  servers: number
  skills: number
  disabled: number
  apps: number
}

export function sectionCounts(input: {
  tools: number
  servers: number
  skills: { enabled: boolean }[]
  apps: number
}): SectionCounts {
  const disabled = input.skills.filter((skill) => !skill.enabled).length
  return {
    tools: input.tools,
    servers: input.servers,
    skills: input.skills.length - disabled,
    disabled,
    apps: input.apps,
  }
}
