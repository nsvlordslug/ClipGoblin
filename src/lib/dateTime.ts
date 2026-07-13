/** Value for a datetime-local input, which expects local time without a zone. */
export function localDateTimeAfter(delayMs: number): string {
  const date = new Date(Date.now() + delayMs)
  const localTime = new Date(date.getTime() - date.getTimezoneOffset() * 60_000)
  return localTime.toISOString().slice(0, 16)
}
