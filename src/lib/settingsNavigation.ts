export const SETTINGS_SECTION_IDS = [
  'account',
  'sources',
  'detection',
  'ai',
  'editing',
  'storage',
  'appearance',
] as const

export type SettingsSectionId = typeof SETTINGS_SECTION_IDS[number]

export type SettingsNavigationKey =
  | 'ArrowDown'
  | 'ArrowLeft'
  | 'ArrowRight'
  | 'ArrowUp'
  | 'End'
  | 'Home'

export function resolveSettingsSection(value: unknown): SettingsSectionId {
  return typeof value === 'string'
    && SETTINGS_SECTION_IDS.includes(value as SettingsSectionId)
    ? value as SettingsSectionId
    : 'account'
}

export function getNextSettingsSection(
  current: SettingsSectionId,
  key: SettingsNavigationKey,
): SettingsSectionId {
  const currentIndex = SETTINGS_SECTION_IDS.indexOf(current)

  if (key === 'Home') return SETTINGS_SECTION_IDS[0]
  if (key === 'End') return SETTINGS_SECTION_IDS[SETTINGS_SECTION_IDS.length - 1]

  const direction = key === 'ArrowRight' || key === 'ArrowDown' ? 1 : -1
  const nextIndex = (currentIndex + direction + SETTINGS_SECTION_IDS.length)
    % SETTINGS_SECTION_IDS.length
  return SETTINGS_SECTION_IDS[nextIndex]
}
