export type SpeechModel = 'base' | 'medium'

interface SpeechModelSettingsPort {
  save: (model: SpeechModel) => Promise<void>
  read: () => Promise<string | null>
}

export async function persistSpeechModelSelection(
  model: SpeechModel,
  settings: SpeechModelSettingsPort,
): Promise<SpeechModel> {
  await settings.save(model)
  const persisted = await settings.read()
  if (persisted !== model) {
    throw new Error('ClipGoblin could not confirm the speech model selection. Please try again.')
  }
  return model
}

export function speechModelLabel(model: string | null | undefined): string | null {
  if (model === 'medium') return 'Quality local (Medium)'
  if (model === 'base') return 'Fast local (Base)'
  return null
}

export function isSpeechModelNavigationLocked(
  saving: boolean,
  destination: string,
): boolean {
  return saving && destination !== '/settings'
}
