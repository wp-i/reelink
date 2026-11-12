import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import test from 'node:test'
import vm from 'node:vm'

const script = readFileSync(
  join(process.cwd(), 'artifacts/extension-tests/background.vm.js'),
  'utf8',
)
const extensionUrl = (path: string) => `chrome-extension://reelink/${path}`
const rect = { x: 4, y: 4, width: 50, height: 50, viewportWidth: 100, viewportHeight: 100 }

function harness() {
  let messageListener: (
    request: unknown,
    sender: unknown,
    respond: (value: any) => void,
  ) => boolean | void
  let activatedListener: () => void
  let removedListener: (id: number) => void
  let updatedListener: (id: number, change: { status?: string }) => void
  let now = 1000
  let nextId = 0
  let activeId = 11
  let offscreenOpen = false
  let capture: any
  let decodeTexts: string[] = ['decoded']
  let contentAck = true
  let viewerUrl: string | undefined
  let viewerId = 98
  let probeHook: (() => void) | undefined
  let captureHook: (() => void) | undefined
  let decodeHook: (() => Promise<void>) | undefined
  const originalTab = { id: 11, windowId: 7, url: 'https://example.org/page', status: 'complete' }
  const calls = {
    offscreen: [] as any[],
    content: [] as any[],
    copies: [] as string[],
    createdTabs: [] as any[],
    removedTabs: [] as number[],
    closes: 0,
    captures: 0,
  }
  const chrome = {
    runtime: {
      id: 'reelink',
      ContextType: { OFFSCREEN_DOCUMENT: 'offscreen' },
      getURL: extensionUrl,
      getContexts: async () => (offscreenOpen ? [{}] : []),
      sendMessage: async (request: any) => {
        if (request.target === 'capture') {
          // A live capture page receives the broadcast; navigated pages do not.
          if (viewerUrl === extensionUrl(`capture.html#${request.sessionId}`))
            await chrome.tabs.remove(viewerId)
          return { ok: true }
        }
        calls.offscreen.push(request)
        if (request.type === 'decode') {
          await decodeHook?.()
          return { ok: true, texts: decodeTexts }
        }
        if (request.type === 'copy') {
          calls.copies.push(request.text)
          return { ok: true, copied: true }
        }
        if (request.type === 'load') return { ok: true, imageDataUrl: 'data:image/png;base64,AAAA' }
        return { ok: true }
      },
      onMessage: {
        addListener(listener: typeof messageListener) {
          messageListener = listener
        },
      },
    },
    storage: {
      session: {
        get: async () => ({
          capture: capture === undefined ? undefined : structuredClone(capture),
        }),
        set: async (value: { capture: unknown }) => {
          capture = structuredClone(value.capture)
        },
        remove: async () => {
          capture = undefined
        },
      },
    },
    tabs: {
      onActivated: {
        addListener(listener: typeof activatedListener) {
          activatedListener = listener
        },
      },
      onRemoved: {
        addListener(listener: typeof removedListener) {
          removedListener = listener
        },
      },
      onUpdated: {
        addListener(listener: typeof updatedListener) {
          updatedListener = listener
        },
      },
      query: async () => [{ ...originalTab, id: activeId }],
      get: async (id: number) => (id === viewerId ? { id } : { ...originalTab }),
      remove: async (id: number) => {
        calls.removedTabs.push(id)
        if (id === viewerId) viewerUrl = undefined
        removedListener(id)
      },
      captureVisibleTab: async () => {
        calls.captures++
        captureHook?.()
        return 'data:image/png;base64,AAAA'
      },
      sendMessage: async (_id: number, message: any, options: any) => {
        calls.content.push({ message, options })
        return message.type === 'start' ? { ok: contentAck } : { ok: true }
      },
      create: async (options: any) => {
        calls.createdTabs.push(options)
        viewerUrl = options.url
        return { id: ++viewerId, ...options }
      },
    },
    scripting: {
      executeScript: async (options: any) => {
        if (options.func && options.target.frameIds) probeHook?.()
        return [{ documentId: 'doc-11', result: true }]
      },
    },
    offscreen: {
      Reason: { CLIPBOARD: 'clipboard', BLOBS: 'blobs' },
      createDocument: async () => {
        offscreenOpen = true
      },
      closeDocument: async () => {
        offscreenOpen = false
        calls.closes++
      },
    },
    action: {
      setBadgeText: async () => {},
      setBadgeBackgroundColor: async () => {},
      setTitle: async () => {},
    },
    commands: { onCommand: { addListener() {} } },
  }

  function boot() {
    class FakeDate extends Date {
      static now() {
        return now
      }
    }
    vm.runInNewContext(script, {
      chrome,
      crypto: { randomUUID: () => `session-${++nextId}` },
      Date: FakeDate,
      console,
    })
  }
  boot()

  const popup = { id: 'reelink', url: extensionUrl('popup.html') }
  const content = {
    id: 'reelink',
    tab: { id: 11 },
    frameId: 0,
    documentId: 'doc-11',
    url: originalTab.url,
  }
  const viewer = (id: string) => ({
    id: 'reelink',
    tab: { id: 99 },
    frameId: 0,
    url: extensionUrl(`capture.html#${id}`),
  })
  const send = (request: any, sender: any = popup): Promise<any> =>
    new Promise((resolve) => {
      if (messageListener(request, sender, resolve) !== true) resolve({ ok: false, ignored: true })
    })
  return {
    calls,
    popup,
    content,
    viewer,
    send,
    boot,
    session: () => capture,
    isOffscreenOpen: () => offscreenOpen,
    setTexts: (texts: string[]) => {
      decodeTexts = texts
    },
    setContentAck: (ok: boolean) => {
      contentAck = ok
    },
    setViewerUrl: (url: string) => {
      viewerUrl = url
    },
    setProbeHook: (hook: () => void) => {
      probeHook = hook
    },
    setCaptureHook: (hook: () => void) => {
      captureHook = hook
    },
    setDecodeHook: (hook: () => Promise<void>) => {
      decodeHook = hook
    },
    switchTab: (id: number) => {
      activeId = id
      activatedListener()
    },
    advance: (millis: number) => {
      now += millis
    },
    removeTab: (id: number) => removedListener(id),
    updateTab: (id: number, status: string) => updatedListener(id, { status }),
  }
}

test('start accepts only the popup and binds the active tab and document', async () => {
  const h = harness()
  assert.equal((await h.send({ target: 'background', type: 'start' }, h.content)).ok, false)
  assert.equal(
    (
      await h.send(
        { target: 'background', type: 'start' },
        { id: 'reelink', url: extensionUrl('other.html') },
      )
    ).ok,
    false,
  )
  assert.equal((await h.send({ target: 'background', type: 'start' })).ok, true)
  assert.equal(h.session().tabId, 11)
  assert.equal(h.session().documentId, 'doc-11')
  assert.equal(h.session().state, 'selecting')
  assert.equal(h.calls.captures, 1)
})

test('content must match extension, tab, top frame, document, and session', async () => {
  const h = harness()
  await h.send({ target: 'background', type: 'start' })
  const request = { target: 'background', type: 'decode', sessionId: h.session().id, rect }
  for (const sender of [
    { ...h.content, id: 'foreign' },
    { ...h.content, tab: { id: 12 } },
    { ...h.content, frameId: 1 },
    { ...h.content, documentId: 'other-doc' },
  ])
    assert.equal((await h.send(request, sender)).ok, false)
  assert.equal((await h.send({ ...request, sessionId: 'wrong' }, h.content)).ok, false)
  assert.equal(h.calls.offscreen.filter((call) => call.type === 'decode').length, 0)
})

test('multi-result decode does not copy; copy accepts only a stored index', async () => {
  const h = harness()
  h.setTexts(['first', 'second'])
  await h.send({ target: 'background', type: 'start' })
  const id = h.session().id
  const decoded = await h.send(
    { target: 'background', type: 'decode', sessionId: id, rect },
    h.content,
  )
  assert.deepEqual(Array.from(decoded.texts), ['first', 'second'])
  assert.equal(decoded.copied, false)
  assert.deepEqual(h.calls.copies, [])
  assert.equal(
    (
      await h.send(
        { target: 'background', type: 'copy', sessionId: id, index: -1, text: 'injected' },
        h.content,
      )
    ).ok,
    false,
  )
  assert.equal(
    (await h.send({ target: 'background', type: 'copy', sessionId: id, index: 2 }, h.content)).ok,
    false,
  )
  assert.equal(
    (
      await h.send(
        { target: 'background', type: 'copy', sessionId: id, index: 1, text: 'injected' },
        h.content,
      )
    ).copied,
    true,
  )
  assert.deepEqual(h.calls.copies, ['second'])
})

test('single-result decode copies exactly its decoded text', async () => {
  const h = harness()
  h.setTexts(['only'])
  await h.send({ target: 'background', type: 'start' })
  const result = await h.send(
    { target: 'background', type: 'decode', sessionId: h.session().id, rect },
    h.content,
  )
  assert.equal(result.copied, true)
  assert.deepEqual(h.calls.copies, ['only'])
})

test('cancel during decode prevents automatic copy before queued cleanup', async () => {
  const h = harness()
  await h.send({ target: 'background', type: 'start' })
  const id = h.session().id
  let releaseDecode!: () => void
  let decodeStarted!: () => void
  const gate = new Promise<void>((resolve) => {
    releaseDecode = resolve
  })
  const started = new Promise<void>((resolve) => {
    decodeStarted = resolve
  })
  h.setDecodeHook(async () => {
    decodeStarted()
    await gate
  })
  const decode = h.send({ target: 'background', type: 'decode', sessionId: id, rect }, h.content)
  await started
  const cancel = h.send({ target: 'background', type: 'cancel', sessionId: id }, h.content)
  releaseDecode()
  assert.equal((await decode).ok, false)
  await cancel
  assert.deepEqual(h.calls.copies, [])
  assert.equal(h.session(), undefined)
})

test('tab navigation during decode prevents automatic copy', async () => {
  const h = harness()
  await h.send({ target: 'background', type: 'start' })
  const id = h.session().id
  let releaseDecode!: () => void
  let decodeStarted!: () => void
  const gate = new Promise<void>((resolve) => {
    releaseDecode = resolve
  })
  const started = new Promise<void>((resolve) => {
    decodeStarted = resolve
  })
  h.setDecodeHook(async () => {
    decodeStarted()
    await gate
  })
  const decode = h.send({ target: 'background', type: 'decode', sessionId: id, rect }, h.content)
  await started
  h.updateTab(11, 'loading')
  releaseDecode()
  assert.equal((await decode).ok, false)
  assert.deepEqual(h.calls.copies, [])
})

test('cancel includes the session ID and releases storage and offscreen document', async () => {
  const h = harness()
  await h.send({ target: 'background', type: 'start' })
  const id = h.session().id
  assert.equal(
    (await h.send({ target: 'background', type: 'cancel', sessionId: id }, h.content)).ok,
    true,
  )
  assert.equal(h.session(), undefined)
  assert.equal(h.isOffscreenOpen(), false)
  assert.equal(h.calls.closes, 1)
  assert.equal(h.calls.content.at(-1).message.type, 'cancel')
  assert.equal(h.calls.content.at(-1).message.sessionId, id)
})

test('service worker reload restores the storage session for an authorized copy', async () => {
  const h = harness()
  h.setTexts(['persisted'])
  await h.send({ target: 'background', type: 'start' })
  const id = h.session().id
  await h.send({ target: 'background', type: 'decode', sessionId: id, rect }, h.content)
  h.boot()
  const result = await h.send(
    { target: 'background', type: 'copy', sessionId: id, index: 0 },
    h.content,
  )
  assert.equal(result.copied, true)
  assert.equal(h.calls.copies.at(-1), 'persisted')
})

test('tab switches before or during capture reject the session', async () => {
  for (const phase of ['probe', 'capture']) {
    const h = harness()
    const hook = () => h.switchTab(12)
    if (phase === 'probe') h.setProbeHook(hook)
    else h.setCaptureHook(hook)
    const result = await h.send({ target: 'background', type: 'start' })
    assert.equal(result.ok, false)
    assert.equal(h.session(), undefined)
    assert.equal(h.calls.offscreen.length, 0)
  }
})

test('failed content acknowledgment opens the captured image viewer', async () => {
  const h = harness()
  h.setContentAck(false)
  assert.equal((await h.send({ target: 'background', type: 'start' })).ok, true)
  const id = h.session().id
  assert.equal(h.session().viewerTabId, 99)
  assert.equal(h.calls.createdTabs[0].url, extensionUrl(`capture.html#${id}`))
  assert.equal(
    (await h.send({ target: 'background', type: 'load', sessionId: id }, h.viewer(id))).ok,
    true,
  )
  assert.equal(
    (await h.send({ target: 'background', type: 'load', sessionId: id }, h.content)).ok,
    false,
  )
})

test('expired sessions reject work and remove retained state', async () => {
  const h = harness()
  await h.send({ target: 'background', type: 'start' })
  const id = h.session().id
  h.advance(120_001)
  const result = await h.send(
    { target: 'background', type: 'decode', sessionId: id, rect },
    h.content,
  )
  assert.equal(result.ok, false)
  assert.match(result.error, /过期/)
  assert.equal(h.session(), undefined)
  assert.equal(h.isOffscreenOpen(), false)
})

test('source navigation or closing releases the fallback screenshot tab', async () => {
  for (const event of ['navigate', 'close']) {
    const h = harness()
    h.setContentAck(false)
    await h.send({ target: 'background', type: 'start' })
    const id = h.session().id
    if (event === 'navigate') h.updateTab(11, 'loading')
    else h.removeTab(11)
    // This request joins the queue after the lifecycle cleanup.
    await h.send({ target: 'background', type: 'load', sessionId: id }, h.viewer(id))
    assert.deepEqual(h.calls.removedTabs, [99])
    assert.equal(h.session(), undefined)
    assert.equal(h.isOffscreenOpen(), false)
  }
})

test('starting a new capture closes the previous fallback screenshot tab', async () => {
  const h = harness()
  h.setContentAck(false)
  await h.send({ target: 'background', type: 'start' })
  const previousId = h.session().id
  assert.equal((await h.send({ target: 'background', type: 'start' })).ok, true)
  assert.deepEqual(h.calls.removedTabs, [99])
  assert.notEqual(h.session().id, previousId)
  assert.equal(h.session().viewerTabId, 100)
})

test('cleanup preserves a fallback tab that the user has navigated elsewhere', async () => {
  const h = harness()
  h.setContentAck(false)
  await h.send({ target: 'background', type: 'start' })
  h.setViewerUrl('https://example.org/another-page')
  await h.send({ target: 'background', type: 'start' })
  assert.deepEqual(h.calls.removedTabs, [])
})

test('offscreen expiry closes the fallback screenshot tab', async () => {
  const h = harness()
  h.setContentAck(false)
  await h.send({ target: 'background', type: 'start' })
  const id = h.session().id
  await h.send(
    { target: 'background', type: 'expired', sessionId: id },
    { id: 'reelink', url: extensionUrl('offscreen.html') },
  )
  assert.deepEqual(h.calls.removedTabs, [99])
  assert.equal(h.session(), undefined)
})
