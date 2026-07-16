export type VodPrimaryActionId =
  | 'analyze'
  | 'analyzing'
  | 'download'
  | 'downloading'
  | 'repair-download'
  | 'retry-analysis'
  | 'view-clips'

export interface VodPrimaryAction {
  id: VodPrimaryActionId
  label: string
  disabled: boolean
  tone: 'accent' | 'busy' | 'danger' | 'warning'
}

export function getVodPrimaryAction({
  analysisStatus,
  downloadStatus,
}: {
  analysisStatus: string
  downloadStatus: string
}): VodPrimaryAction {
  if (analysisStatus === 'completed') {
    return { id: 'view-clips', label: 'View clips', disabled: false, tone: 'accent' }
  }
  if (analysisStatus === 'analyzing') {
    return { id: 'analyzing', label: 'Analyzing', disabled: true, tone: 'busy' }
  }
  if (downloadStatus === 'failed') {
    return { id: 'repair-download', label: 'Repair & retry', disabled: false, tone: 'warning' }
  }
  if (downloadStatus === 'downloading') {
    return { id: 'downloading', label: 'Downloading', disabled: true, tone: 'busy' }
  }
  if (downloadStatus !== 'downloaded') {
    return { id: 'download', label: 'Download', disabled: false, tone: 'accent' }
  }
  if (analysisStatus === 'failed') {
    return { id: 'retry-analysis', label: 'Retry analysis', disabled: false, tone: 'danger' }
  }
  return { id: 'analyze', label: 'Analyze', disabled: false, tone: 'accent' }
}
