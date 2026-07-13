export function parseStoredTags(value: unknown): string[] {
  let tags: unknown[]

  if (Array.isArray(value)) {
    tags = value
  } else if (typeof value === 'string') {
    const trimmed = value.trim()
    if (!trimmed) return []

    try {
      const parsed: unknown = JSON.parse(trimmed)
      tags = Array.isArray(parsed) ? parsed : trimmed.split(',')
    } catch {
      tags = trimmed.split(',')
    }
  } else {
    return []
  }

  return [...new Set(
    tags
      .filter((tag): tag is string => typeof tag === 'string')
      .map(tag => tag.trim().toLowerCase())
      .filter(Boolean),
  )]
}
