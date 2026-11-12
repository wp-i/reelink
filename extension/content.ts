import { mountSelection, type SelectionHandle } from './selection'
import type { BackgroundRequest, CaptureStart, OperationResponse, SelectionRect } from './shared'

type ContentMessage = CaptureStart | { target: 'content'; type: 'cancel'; sessionId: string }

const scope = globalThis as typeof globalThis & { __reelinkContentInstalled?: boolean }
if (!scope.__reelinkContentInstalled) {
  scope.__reelinkContentInstalled = true
  let current: { sessionId: string; handle: SelectionHandle } | null = null

  chrome.runtime.onMessage.addListener((raw: ContentMessage, sender, respond) => {
    if (
      sender.id !== chrome.runtime.id ||
      sender.tab ||
      (sender.url !== undefined && sender.url !== chrome.runtime.getURL('background.js'))
    )
      return
    if (!raw || raw.target !== 'content') return
    if (raw.type === 'cancel') {
      if (current?.sessionId === raw.sessionId) current.handle.close(false)
      respond({ ok: true } satisfies OperationResponse)
      return
    }
    if (raw.type !== 'start') return
    current?.handle.close()
    try {
      const sessionId = raw.sessionId
      const request = async (
        type: 'decode' | 'copy' | 'cancel',
        value?: SelectionRect | number,
      ): Promise<OperationResponse> => {
        let message: BackgroundRequest
        if (type === 'decode')
          message = { target: 'background', type, sessionId, rect: value as SelectionRect }
        else if (type === 'copy')
          message = { target: 'background', type, sessionId, index: value as number }
        else message = { target: 'background', type, sessionId }
        return chrome.runtime.sendMessage(message) as Promise<OperationResponse>
      }
      const handle = mountSelection(raw, {
        fit: 'viewport',
        request,
        onClose: () => {
          if (current?.sessionId === sessionId) current = null
        },
      })
      current = { sessionId, handle }
      void handle.ready.then(
        () => respond({ ok: true } satisfies OperationResponse),
        () =>
          respond({ ok: false, error: '当前页面无法显示截图选区。' } satisfies OperationResponse),
      )
      return true
    } catch {
      respond({
        ok: false,
        error: '无法显示截图选区，请刷新页面重试。',
      } satisfies OperationResponse)
    }
  })
}
