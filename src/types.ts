export type View = 'qr' | 'rename' | 'settings'
export type RenameKind = 'movie' | 'tv'

export interface QrResult {
  texts: string[]
  imageDataUrl: string
}

export interface RenameItem {
  id: string
  sourcePath: string
  originalName: string
  suggestedName: string
  title: string
  year: string | null
  extension: string
  kind: RenameKind
  confidence: 'high' | 'review'
  notes: string[]
  isDirectory: boolean
  groupId: string | null
  episode: string | null
}

export interface RenameEdit {
  id: string
  sourcePath: string
  targetName: string
}

export interface RenameOutcome {
  count: number
  message: string
}

export interface HistorySummary {
  count: number
  createdAt: string
}

export interface TmdbMatch {
  id: number
  title: string
  originalTitle: string
  year: string | null
  overview: string
  kind: RenameKind
}

export interface PreviewRow extends RenameItem {
  selected: boolean
  targetName: string
}
