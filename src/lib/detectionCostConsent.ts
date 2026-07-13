export type DetectionSensitivity = 'low' | 'medium' | 'high'

interface HighDetectionConsentInput {
  currentSensitivity: DetectionSensitivity
  nextSensitivity: DetectionSensitivity
  byokProviderSelected: boolean
}

export function requiresHighDetectionCostConsent({
  currentSensitivity,
  nextSensitivity,
  byokProviderSelected,
}: HighDetectionConsentInput): boolean {
  return byokProviderSelected
    && currentSensitivity !== 'high'
    && nextSensitivity === 'high'
}
