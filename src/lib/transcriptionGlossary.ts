export const normalizeTranscriptionGlossary = (value: string): string => {
  const seen = new Set<string>()
  const terms: string[] = []
  for (const source of value.replace(/[\u2018\u2019]/g, "'").split(/[,;\r\n]+/)) {
    const term = Array.from(source.replace(/\0/g, ' ').trim().replace(/\s+/g, ' '))
      .slice(0, 48)
      .join('')
    const key = term.toLocaleLowerCase()
    if (!term || seen.has(key)) continue
    seen.add(key)
    terms.push(term)
    if (terms.length === 24) break
  }
  return terms.join(', ')
}

export const mergeTranscriptionGlossaryTerms = (
  current: string,
  additions: string[],
): string => normalizeTranscriptionGlossary([current, ...additions].filter(Boolean).join(', '))
