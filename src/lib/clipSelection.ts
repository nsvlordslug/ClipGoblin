interface SelectableClip {
  id: string
  render_status: string
  output_path: string | null
}

export function filterCompletedClipGroups<T extends { clips: SelectableClip[] }>(
  groups: T[],
  hideCompleted: boolean,
): T[] {
  return hideCompleted
    ? groups.filter(group => !group.clips.every(clip => clip.render_status === 'completed' && clip.output_path))
    : groups
}

export function retainDisplayedSelection(
  selectedIds: Set<string>,
  displayedClips: Array<{ id: string }>,
): Set<string> {
  const displayedIds = new Set(displayedClips.map(clip => clip.id))
  const retained = new Set([...selectedIds].filter(id => displayedIds.has(id)))
  return retained.size === selectedIds.size ? selectedIds : retained
}
