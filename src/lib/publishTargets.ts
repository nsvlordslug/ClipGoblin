interface PublishAccount { account_id: string }

export function uploadAdapterPlatform(platform: string): string {
  return platform === 'youtube_shorts' ? 'youtube' : platform
}

/** Copy IDs before exporting so an account switch cannot retarget this action. */
export function captureUploadTargets(
  platforms: string[],
  accounts: Record<string, PublishAccount | null>,
): Record<string, string | null> {
  return Object.fromEntries(platforms.map(platform => {
    const adapter = uploadAdapterPlatform(platform)
    return [adapter, accounts[adapter]?.account_id || null]
  }))
}

export function uploadTargetFields(accountId: string | null | undefined) {
  if (!accountId) throw new Error('Connect the publishing account and try again.')
  return { target_account_id: accountId }
}

export function isUncertainUploadError(error: unknown): boolean {
  return String(error).toLowerCase().includes('upload outcome is uncertain')
}
