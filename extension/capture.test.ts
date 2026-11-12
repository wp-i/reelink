import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import vm from 'node:vm'
import { transformSync } from 'esbuild'

const source = readFileSync('extension/capture.ts', 'utf8').replace(/^import .*\r?\n/gm, '')
const script = transformSync(source, { loader: 'ts', target: 'chrome116' }).code
const getURL = (path: string) => `chrome-extension://reelink/${path}`

function harness() {
  let listener: (message: any, sender: any, respond: (response: unknown) => void) => void
  let finishLoad!: (response: unknown) => void
  let finishReady!: () => void
  let mounted = 0
  const removed: number[] = []
  const closed: boolean[] = []
  const ready = new Promise<void>((resolve) => {
    finishReady = resolve
  })
  const load = new Promise((resolve) => {
    finishLoad = resolve
  })
  vm.runInNewContext(script, {
    location: { hash: '#session-1' },
    document: { getElementById: () => ({ remove() {} }) },
    window: { close() {} },
    chrome: {
      runtime: {
        id: 'reelink',
        getURL,
        sendMessage: () => load,
        onMessage: {
          addListener: (callback: typeof listener) => {
            listener = callback
          },
        },
      },
      tabs: {
        // No tabs permission: only the current tab ID is available.
        getCurrent: async () => ({ id: 99 }),
        remove: async (id: number) => {
          removed.push(id)
        },
      },
    },
    mountSelection: () => {
      mounted++
      return {
        ready,
        close: (notify: boolean) => {
          closed.push(notify)
        },
      }
    },
  })
  return {
    removed,
    closed,
    mounted: () => mounted,
    finishLoad: () => finishLoad({ ok: true, imageDataUrl: 'data:image/png;base64,AAAA' }),
    finishReady,
    cancel: (sessionId = 'session-1', sender: any = { id: 'reelink' }) =>
      listener({ target: 'capture', type: 'cancel', sessionId }, sender, () => {}),
  }
}

test('fallback cancellation before image load closes without mounting a screenshot', async () => {
  const h = harness()
  h.cancel()
  h.finishLoad()
  await new Promise(setImmediate)
  assert.deepEqual(h.removed, [99])
  assert.equal(h.mounted(), 0)
})

test('fallback cancellation releases a mounted screenshot without tabs URL access', async () => {
  const h = harness()
  h.finishLoad()
  await new Promise(setImmediate)
  h.cancel()
  h.finishReady()
  await new Promise(setImmediate)
  assert.deepEqual(h.closed, [false])
  assert.deepEqual(h.removed, [99])
})

test('fallback rejects foreign sessions and non-worker cancellation senders', async () => {
  const h = harness()
  h.cancel('another-session')
  h.cancel('session-1', { id: 'other' })
  h.cancel('session-1', { id: 'reelink', tab: { id: 11 } })
  h.cancel('session-1', { id: 'reelink', url: getURL('popup.html') })
  await new Promise(setImmediate)
  assert.deepEqual(h.removed, [])
  h.cancel('session-1', { id: 'reelink', url: getURL('background.js') })
  await new Promise(setImmediate)
  assert.deepEqual(h.removed, [99])
})
