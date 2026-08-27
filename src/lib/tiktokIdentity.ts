export interface TikTokIdentityView {
  verified: boolean
  primary: string
  secondary: string | null
}

export function describeTikTokIdentity(
  creatorUsername: string | null | undefined,
  creatorNickname: string | null | undefined,
): TikTokIdentityView {
  const username = creatorUsername?.trim().replace(/^@+/, '').trim() ?? ''
  const nickname = creatorNickname?.trim() ?? ''

  if (!username) {
    return {
      verified: false,
      primary: 'TikTok identity unverified',
      secondary: nickname || null,
    }
  }

  return {
    verified: true,
    primary: `@${username}`,
    secondary: nickname && nickname.toLowerCase() !== username.toLowerCase()
      ? nickname
      : null,
  }
}
