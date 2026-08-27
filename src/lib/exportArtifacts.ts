export interface RenderedArtifact {
  path: string
  revision: string
  aspectRatio: string
  width: number
  height: number
}

export interface ArtifactUploadFields {
  artifact_path: string
  artifact_revision: string
  artifact_aspect_ratio: string
}

export function artifactUploadFields(artifact: RenderedArtifact): ArtifactUploadFields {
  return {
    artifact_path: artifact.path,
    artifact_revision: artifact.revision,
    artifact_aspect_ratio: artifact.aspectRatio,
  }
}

export async function saveThenRender<T>(
  saveSnapshot: () => Promise<void>,
  renderSnapshot: () => Promise<T>,
): Promise<T> {
  await saveSnapshot()
  return renderSnapshot()
}

export function renderSnapshotKey(
  targetAspectRatio: string,
  settings: Record<string, unknown>,
): string {
  return JSON.stringify({ targetAspectRatio, settings })
}
