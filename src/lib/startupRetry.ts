export const STARTUP_RETRY_DELAYS_MS = [0, 100, 250, 500, 1_000, 2_000] as const

export async function withStartupRetry<T>(
  operation: () => Promise<T>,
  delays: readonly number[] = STARTUP_RETRY_DELAYS_MS,
): Promise<T> {
  let lastError: unknown

  for (const delay of delays) {
    if (delay > 0) {
      await new Promise(resolve => setTimeout(resolve, delay))
    }

    try {
      return await operation()
    } catch (error) {
      lastError = error
    }
  }

  throw lastError
}
