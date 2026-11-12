import {
  SESSION_TTL,
  type BackgroundRequest,
  type OffscreenRequest,
  type OperationResponse,
} from './shared'

interface Session {
  id: string
  tabId: number
  windowId: number
  documentId?: string
  viewerTabId?: number
  createdAt: number
  texts: string[]
  state: 'selecting' | 'result'
}

let queue: Promise<unknown> = Promise.resolve()
let creating: Promise<void> | undefined
let activationVersion = 0
let activeSession: Session | undefined
const cancelled = new Set<string>()
chrome.tabs.onActivated.addListener(() => {
  activationVersion += 1
})
function serial<T>(task: () => Promise<T>): Promise<T> {
  const result = queue.then(task, task)
  queue = result.catch(() => undefined)
  return result
}

async function currentSession(): Promise<Session | undefined> {
  const session: Session | undefined = (await chrome.storage.session.get('capture')).capture
  activeSession = session
  return session
}

async function ensureOffscreen() {
  if (creating) return creating
  if (
    (
      await chrome.runtime.getContexts({
        contextTypes: [chrome.runtime.ContextType.OFFSCREEN_DOCUMENT],
      })
    ).length
  )
    return
  creating = chrome.offscreen.createDocument({
    url: 'offscreen.html',
    reasons: [chrome.offscreen.Reason.CLIPBOARD, chrome.offscreen.Reason.BLOBS],
    justification: '在本机裁剪截图、识别二维码并复制用户选择的结果',
  })
  try {
    await creating
  } finally {
    creating = undefined
  }
}

async function offscreen(request: OffscreenRequest): Promise<OperationResponse> {
  await ensureOffscreen()
  const result: OperationResponse = await chrome.runtime.sendMessage(request)
  if (!result?.ok) throw new Error(result?.error || '本地处理失败，请重试。')
  return result
}

async function closeOffscreen() {
  if (
    (
      await chrome.runtime.getContexts({
        contextTypes: [chrome.runtime.ContextType.OFFSCREEN_DOCUMENT],
      })
    ).length
  )
    await chrome.offscreen.closeDocument()
}

async function clearSession() {
  const session = await currentSession()
  activeSession = undefined
  await chrome.storage.session.remove('capture')
  if (session) cancelled.delete(session.id)
  await closeOffscreen()
  if (session?.viewerTabId !== undefined) {
    // Extension pages receive runtime messages without requiring tabs/host access.
    await chrome.runtime
      .sendMessage({ target: 'capture', type: 'cancel', sessionId: session.id })
      .catch(() => undefined)
  }
  if (session?.documentId) {
    await chrome.tabs
      .sendMessage(
        session.tabId,
        { target: 'content', type: 'cancel', sessionId: session.id },
        { documentId: session.documentId },
      )
      .catch(() => undefined)
  }
}

async function assertActive(tabId: number, windowId: number) {
  const [active] = await chrome.tabs.query({ active: true, windowId })
  if (active?.id !== tabId) throw new Error('标签页已切换，请回到需要截图的页面后重试。')
}

async function start(tab?: chrome.tabs.Tab): Promise<OperationResponse> {
  const previous = await currentSession()
  if (previous) {
    await clearSession()
    // A second shortcut press dismisses the current overlay before another capture.
    if (previous.tabId === tab?.id && Date.now() - previous.createdAt < SESSION_TTL)
      return { ok: true }
  }
  if (!tab) [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true })
  if (tab?.id === undefined || tab.windowId === undefined)
    throw new Error('请先打开需要识别的网页。')
  const tabId = tab.id
  const windowId = tab.windowId
  await chrome.action.setBadgeText({ text: '', tabId })
  let documentId: string | undefined
  try {
    const [probe] = await chrome.scripting.executeScript({
      target: { tabId, frameIds: [0] },
      func: () => location.href,
    })
    documentId = probe.documentId
  } catch {
    /* Chrome internal pages use the extension screenshot page. */
  }
  await assertActive(tabId, windowId)
  const capturedActivation = activationVersion
  const imageDataUrl = await chrome.tabs.captureVisibleTab(windowId, { format: 'png' })
  await assertActive(tabId, windowId)
  if (activationVersion !== capturedActivation) throw new Error('截图时标签页已切换，请重试。')
  const after = await chrome.tabs.get(tabId)
  if (after.url !== tab.url || after.status === 'loading')
    throw new Error('页面已跳转，请等待加载完成后重试。')
  if (documentId)
    await chrome.scripting.executeScript({
      target: { tabId, documentIds: [documentId] },
      func: () => true,
    })
  const session: Session = {
    id: crypto.randomUUID(),
    tabId,
    windowId,
    documentId,
    createdAt: Date.now(),
    texts: [],
    state: 'selecting',
  }
  try {
    await offscreen({ target: 'offscreen', type: 'prepare', sessionId: session.id, imageDataUrl })
    await chrome.storage.session.set({ capture: session })
    activeSession = session
    if (documentId) {
      try {
        await chrome.scripting.executeScript({
          target: { tabId, documentIds: [documentId] },
          files: ['content.js'],
        })
        const mounted: OperationResponse = await chrome.tabs.sendMessage(
          tabId,
          { target: 'content', type: 'start', sessionId: session.id, imageDataUrl },
          { documentId },
        )
        if (!mounted?.ok) throw new Error('无法显示页面选区。')
        return { ok: true }
      } catch {
        /* Fall back if this page cannot host the selector. */
      }
    }
    const viewer = await chrome.tabs.create({
      url: chrome.runtime.getURL(`capture.html#${session.id}`),
      active: true,
      windowId,
    })
    session.viewerTabId = viewer.id
    await chrome.storage.session.set({ capture: session })
    return { ok: true }
  } catch (error) {
    await clearSession()
    throw error
  }
}

function isViewer(sender: chrome.runtime.MessageSender, session: Session) {
  return (
    sender.tab?.id === session.viewerTabId &&
    sender.frameId === 0 &&
    sender.url === chrome.runtime.getURL(`capture.html#${session.id}`)
  )
}

function isOwner(sender: chrome.runtime.MessageSender, session: Session) {
  return (
    isViewer(sender, session) ||
    (sender.tab?.id === session.tabId &&
      sender.frameId === 0 &&
      !!session.documentId &&
      sender.documentId === session.documentId)
  )
}

async function handle(
  request: BackgroundRequest,
  sender: chrome.runtime.MessageSender,
): Promise<OperationResponse> {
  if (request.type === 'start') {
    if (sender.tab || sender.url !== chrome.runtime.getURL('popup.html'))
      throw new Error('请使用快捷键或工具栏按钮开始截图。')
    return start()
  }
  const session = await currentSession()
  if (!session || session.id !== request.sessionId) throw new Error('截图已结束，请重新按 Alt+Q。')
  if (request.type === 'expired') {
    if (sender.tab || sender.url !== chrome.runtime.getURL('offscreen.html'))
      throw new Error('无效请求。')
    await clearSession()
    return { ok: true }
  }
  if (!isOwner(sender, session)) throw new Error('无法访问这次截图。')
  activeSession = session
  if (Date.now() - session.createdAt > SESSION_TTL) {
    await clearSession()
    throw new Error('截图已过期，请重新按 Alt+Q。')
  }
  switch (request.type) {
    case 'cancel':
      await clearSession()
      return { ok: true }
    case 'load':
      if (!isViewer(sender, session) || session.state !== 'selecting')
        throw new Error('截图已结束，请重新按 Alt+Q。')
      return offscreen({ target: 'offscreen', type: 'load', sessionId: session.id })
    case 'decode': {
      session.state = 'result'
      session.texts = []
      await chrome.storage.session.set({ capture: session })
      try {
        const result = await offscreen({
          target: 'offscreen',
          type: 'decode',
          sessionId: session.id,
          rect: request.rect,
        })
        if (cancelled.has(session.id)) return { ok: false, error: '截图已取消。' }
        session.texts = result.texts || []
        await chrome.storage.session.set({ capture: session })
        if (cancelled.has(session.id)) return { ok: false, error: '截图已取消。' }
        let copied = false
        if (session.texts.length === 1)
          copied =
            (await offscreen({ target: 'offscreen', type: 'copy', text: session.texts[0] }))
              .copied === true
        return { ok: true, texts: session.texts, copied }
      } catch (error) {
        throw error
      }
    }
    case 'copy': {
      if (cancelled.has(session.id)) throw new Error('截图已取消。')
      if (
        !Number.isInteger(request.index) ||
        request.index < 0 ||
        request.index >= session.texts.length
      )
        throw new Error('请选择已识别的结果。')
      const result = await offscreen({
        target: 'offscreen',
        type: 'copy',
        text: session.texts[request.index],
      })
      return { ok: true, copied: result.copied }
    }
  }
}

chrome.runtime.onMessage.addListener((request, sender, respond) => {
  if (sender.id !== chrome.runtime.id || request?.target !== 'background') return
  // Mark an authorized cancellation immediately, even while decoding is queued.
  const session = activeSession
  if (
    request.type === 'cancel' &&
    session &&
    session.id === request.sessionId &&
    isOwner(sender, session)
  )
    cancelled.add(session.id)
  void serial(() => handle(request, sender)).then(respond, (error: unknown) =>
    respond({
      ok: false,
      error: error instanceof Error ? error.message : '截图识别失败，请重试。',
    }),
  )
  return true
})

chrome.commands.onCommand.addListener((command, tab) => {
  if (command !== 'capture-qr') return
  void serial(() => start(tab)).catch(async (error: unknown) => {
    const tabId = tab?.id
    const message = error instanceof Error ? error.message : '截图失败，请重试。'
    await chrome.action.setBadgeBackgroundColor({ color: '#a23b40', tabId })
    await chrome.action.setBadgeText({ text: '!', tabId })
    await chrome.action.setTitle({ title: `Reelink：${message}`, tabId })
  })
})

chrome.tabs.onRemoved.addListener((tabId) => {
  if (activeSession && (activeSession.tabId === tabId || activeSession.viewerTabId === tabId))
    cancelled.add(activeSession.id)
  void serial(async () => {
    const session = await currentSession()
    if (session && (session.tabId === tabId || session.viewerTabId === tabId)) await clearSession()
  })
})

chrome.tabs.onUpdated.addListener((tabId, change) => {
  if (change.status !== 'loading') return
  if (activeSession?.tabId === tabId) cancelled.add(activeSession.id)
  void serial(async () => {
    const session = await currentSession()
    if (session?.tabId === tabId) await clearSession()
  })
})
