export class LatestRequestGate {
  private generation = 0

  begin(): number {
    this.generation += 1
    return this.generation
  }

  isCurrent(requestGeneration: number): boolean {
    return requestGeneration === this.generation
  }

  cancel(requestGeneration: number): void {
    if (this.isCurrent(requestGeneration)) this.generation += 1
  }
}

export function canPersistEditorState(
  routeClipId: string | undefined,
  loadedClipId: string | null,
): routeClipId is string {
  return typeof routeClipId === 'string' && routeClipId.length > 0 && routeClipId === loadedClipId
}
