export const SESSION_TTL = 120_000

export interface SelectionRect {
  x: number
  y: number
  width: number
  height: number
  viewportWidth: number
  viewportHeight: number
}

export interface CaptureStart {
  target: 'content'
  type: 'start'
  sessionId: string
  imageDataUrl: string
}

export interface OperationResponse {
  ok: boolean
  error?: string
  texts?: string[]
  copied?: boolean
  imageDataUrl?: string
  sessionId?: string
}

export type BackgroundRequest =
  | { target: 'background'; type: 'start' }
  | { target: 'background'; type: 'load'; sessionId: string }
  | { target: 'background'; type: 'decode'; sessionId: string; rect: SelectionRect }
  | { target: 'background'; type: 'copy'; sessionId: string; index: number }
  | { target: 'background'; type: 'cancel'; sessionId: string }
  | { target: 'background'; type: 'expired'; sessionId: string }

export type OffscreenRequest =
  | { target: 'offscreen'; type: 'prepare'; sessionId: string; imageDataUrl: string }
  | { target: 'offscreen'; type: 'load'; sessionId: string }
  | { target: 'offscreen'; type: 'decode'; sessionId: string; rect: SelectionRect }
  | { target: 'offscreen'; type: 'copy'; text: string }
