import { decodePixels } from './decode'
import { cropRect } from './geometry'
import { SESSION_TTL, type OffscreenRequest, type OperationResponse } from './shared'

let capture: { id: string; image: string } | undefined
let expiry: ReturnType<typeof setTimeout> | undefined

function release() {
  capture = undefined
  clearTimeout(expiry)
}

function copyText(text: string): boolean {
  const input = document.createElement('textarea')
  input.value = text
  document.body.append(input)
  input.select()
  try {
    return document.execCommand('copy')
  } finally {
    input.value = ''
    input.remove()
  }
}

async function handle(request: OffscreenRequest): Promise<OperationResponse> {
  if (request.type === 'prepare') {
    release()
    if (
      !request.imageDataUrl.startsWith('data:image/png;base64,') ||
      request.imageDataUrl.length > 32_000_000
    ) {
      throw new Error('截图过大，请缩小浏览器窗口后重试。')
    }
    capture = { id: request.sessionId, image: request.imageDataUrl }
    expiry = setTimeout(() => {
      release()
      void chrome.runtime.sendMessage({
        target: 'background',
        type: 'expired',
        sessionId: request.sessionId,
      })
    }, SESSION_TTL)
    return { ok: true }
  }
  if (request.type === 'copy') return { ok: true, copied: copyText(request.text) }
  if (!capture || capture.id !== request.sessionId) throw new Error('截图已过期，请重新按 Alt+Q。')
  if (request.type === 'load')
    return { ok: true, imageDataUrl: capture.image, sessionId: capture.id }
  const dataUrl = capture.image
  // Blob and canvas are local; connect-src blocks any network access.
  const encoded = atob(dataUrl.slice(dataUrl.indexOf(',') + 1))
  const bytes = Uint8Array.from(encoded, (char) => char.charCodeAt(0))
  const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }))
  try {
    if (bitmap.width * bitmap.height > 20_000_000)
      throw new Error('截图尺寸过大，请缩小浏览器窗口后重试。')
    const rect = cropRect(request.rect, bitmap.width, bitmap.height)
    const canvas = document.createElement('canvas')
    canvas.width = rect.width
    canvas.height = rect.height
    const context = canvas.getContext('2d', { willReadFrequently: true })
    if (!context) throw new Error('无法读取截图，请重试。')
    context.drawImage(
      bitmap,
      rect.x,
      rect.y,
      rect.width,
      rect.height,
      0,
      0,
      rect.width,
      rect.height,
    )
    const pixels = context.getImageData(0, 0, rect.width, rect.height)
    const texts = decodePixels(pixels.data, rect.width, rect.height)
    canvas.width = canvas.height = 0
    return { ok: true, texts }
  } finally {
    bitmap.close()
  }
}

chrome.runtime.onMessage.addListener((request, sender, respond) => {
  if (
    sender.id !== chrome.runtime.id ||
    sender.tab ||
    (sender.url !== undefined && sender.url !== chrome.runtime.getURL('background.js')) ||
    request?.target !== 'offscreen'
  )
    return
  void handle(request).then(respond, (error: unknown) =>
    respond({
      ok: false,
      error: error instanceof Error ? error.message : '本地识别失败，请重试。',
    }),
  )
  return true
})
