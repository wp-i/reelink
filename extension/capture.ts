import { mountSelection, type SelectionHandle } from './selection'
import type { BackgroundRequest, OperationResponse, SelectionRect } from './shared'

const loading = document.getElementById('loading')!
let sessionId = ''
try {
  sessionId = decodeURIComponent(location.hash.slice(1))
} catch {
  /* Invalid URL is handled below. */
}
let cancelled = false
let closing = false
let selection: SelectionHandle | undefined
chrome.runtime.onMessage.addListener((message, sender, respond) => {
  if (
    sender.id !== chrome.runtime.id ||
    sender.tab ||
    (sender.url !== undefined && sender.url !== chrome.runtime.getURL('background.js')) ||
    message?.target !== 'capture' ||
    message.type !== 'cancel' ||
    message.sessionId !== sessionId
  )
    return
  cancelled = true
  selection?.close(false)
  void closeTab()
  respond({ ok: true })
})

async function closeTab() {
  if (closing) return
  closing = true
  try {
    const tab = await chrome.tabs.getCurrent()
    if (tab?.id !== undefined) {
      await chrome.tabs.remove(tab.id)
      return
    }
  } catch {
    /* Fall back to ordinary window closing. */
  }
  window.close()
}

function showError(message: string) {
  loading.textContent = message
  const close = document.createElement('button')
  close.textContent = '关闭'
  close.addEventListener('click', () => {
    void closeTab()
  })
  loading.append(document.createElement('br'), close)
}

async function request(
  type: 'decode' | 'copy' | 'cancel',
  value?: SelectionRect | number,
): Promise<OperationResponse> {
  let message: BackgroundRequest
  if (type === 'decode')
    message = { target: 'background', type, sessionId, rect: value as SelectionRect }
  else if (type === 'copy')
    message = { target: 'background', type, sessionId, index: value as number }
  else message = { target: 'background', type, sessionId }
  return chrome.runtime.sendMessage(message) as Promise<OperationResponse>
}

async function init() {
  if (!sessionId) {
    showError('截图会话无效，请重新截图。')
    return
  }
  try {
    const response = (await chrome.runtime.sendMessage({
      target: 'background',
      type: 'load',
      sessionId,
    } satisfies BackgroundRequest)) as OperationResponse
    if (cancelled) return
    if (!response.ok || !response.imageDataUrl) {
      showError(response.error || '截图已过期，请重新截图。')
      return
    }
    let active = false
    const handle = mountSelection(
      { sessionId, imageDataUrl: response.imageDataUrl },
      {
        fit: 'contain',
        request,
        onClose: () => {
          if (active) void closeTab()
        },
      },
    )
    selection = handle
    await handle.ready
    if (cancelled) return
    active = true
    loading.remove()
  } catch {
    if (!cancelled) showError('无法载入截图，请重新截图。')
  }
}

void init()
