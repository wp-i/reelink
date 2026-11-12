import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { ArrowLeft, ArrowRight, House, RotateCw, X } from 'lucide-react'
import './resource-browser.css'

interface BrowserState {
  url: string
  title: string
  loading: boolean
  canGoBack: boolean
  canGoForward: boolean
}

type BrowserAction = 'back' | 'forward' | 'reload' | 'stop' | 'home' | 'navigate'

function navigationUrl(value: string): string {
  const input = value.trim()
  if (!input || /[\s\\\u0000-\u001f\u007f]/.test(input)) {
    throw new Error('请输入有效的网站地址。')
  }
  if (/^[a-z][a-z\d+.-]*:/i.test(input) && !/^https?:\/\//i.test(input)) {
    // A host with a numeric port may omit the protocol.
    if (!/^[^/:]+:\d+(?:[/?#]|$)/.test(input)) {
      throw new Error('仅支持 http:// 或 https:// 网站地址。')
    }
  }
  const normalized = /^https?:\/\//i.test(input) ? input : `https://${input}`
  let parsed: URL
  try {
    parsed = new URL(normalized)
  } catch {
    throw new Error('请输入有效的网站地址。')
  }
  if (
    !['http:', 'https:'].includes(parsed.protocol) ||
    !parsed.hostname ||
    parsed.username ||
    parsed.password
  ) {
    throw new Error('仅支持不含账号信息的 http:// 或 https:// 网站地址。')
  }
  return parsed.href
}

export default function ResourceBrowser() {
  const [state, setState] = useState<BrowserState | null>(null)
  const [draft, setDraft] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const address = useRef<HTMLInputElement>(null)
  const editing = useRef(false)
  const stateRef = useRef<BrowserState | null>(null)
  const busyRef = useRef(false)
  const eventVersion = useRef(0)

  const acceptState = useCallback((next: BrowserState) => {
    stateRef.current = next
    setState(next)
    if (!editing.current) setDraft(next.url)
    setError('')
  }, [])

  const readState = useCallback(async () => {
    const before = eventVersion.current
    try {
      const next = await invoke<BrowserState>('resource_browser_state')
      if (eventVersion.current === before) acceptState(next)
    } catch (reason) {
      if (eventVersion.current === before) setError(String(reason))
    }
  }, [acceptState])

  const act = useCallback(
    async (action: BrowserAction, url?: string) => {
      if (busyRef.current) return
      busyRef.current = true
      setBusy(true)
      setError('')
      try {
        await invoke('resource_browser_action', { action, ...(url ? { url } : {}) })
        // The backend emits later page-load changes; this also catches immediate navigation state.
        await readState()
      } catch (reason) {
        setError(String(reason))
      } finally {
        busyRef.current = false
        setBusy(false)
      }
    },
    [readState],
  )

  useEffect(() => {
    let active = true
    const webview = getCurrentWebview()
    const subscriptions = [
      webview.listen<BrowserState>('resource-browser-state', (event) => {
        if (active) {
          eventVersion.current += 1
          acceptState(event.payload)
        }
      }),
      webview.listen('resource-browser-focus-address', () => {
        if (!active) return
        address.current?.focus()
        address.current?.select()
      }),
    ]
    void Promise.allSettled(subscriptions).then((results) => {
      if (active) {
        results.forEach((result) => {
          if (result.status === 'rejected') setError(String(result.reason))
        })
        void readState()
      }
    })
    return () => {
      active = false
      void Promise.allSettled(subscriptions).then((results) => {
        results.forEach((result) => {
          if (result.status === 'fulfilled') result.value()
        })
      })
    }
  }, [acceptState, readState])

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.ctrlKey && !event.altKey && !event.metaKey) {
        if (event.key.toLowerCase() === 'l') {
          event.preventDefault()
          address.current?.focus()
          address.current?.select()
        } else if (event.key.toLowerCase() === 'r') {
          event.preventDefault()
          if (stateRef.current) void act('reload')
        }
      } else if (event.key === 'F5') {
        event.preventDefault()
        if (stateRef.current) void act('reload')
      } else if (event.key === 'Escape' && !editing.current && stateRef.current?.loading) {
        event.preventDefault()
        void act('stop')
      } else if (event.altKey && !event.ctrlKey && !event.metaKey) {
        if (event.key === 'ArrowLeft' && stateRef.current?.canGoBack) {
          event.preventDefault()
          void act('back')
        } else if (event.key === 'ArrowRight' && stateRef.current?.canGoForward) {
          event.preventDefault()
          void act('forward')
        }
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [act])

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    try {
      const url = navigationUrl(draft)
      editing.current = false
      address.current?.blur()
      void act('navigate', url)
    } catch (reason) {
      setError(String(reason))
      address.current?.focus()
    }
  }

  return (
    <main className="resource-browser" aria-label="资源浏览器工具栏">
      <nav className="resource-browser-controls" aria-label="浏览导航">
        <button
          type="button"
          aria-label="返回"
          title="返回 (Alt+←)"
          disabled={!state?.canGoBack || busy}
          onClick={() => void act('back')}
        >
          <ArrowLeft size={17} strokeWidth={2} />
        </button>
        <button
          type="button"
          aria-label="前进"
          title="前进 (Alt+→)"
          disabled={!state?.canGoForward || busy}
          onClick={() => void act('forward')}
        >
          <ArrowRight size={17} strokeWidth={2} />
        </button>
        <button
          type="button"
          aria-label={state?.loading ? '停止加载' : '刷新'}
          title={state?.loading ? '停止加载' : '刷新 (Ctrl+R / F5)'}
          disabled={!state || busy}
          onClick={() => void act(state?.loading ? 'stop' : 'reload')}
        >
          {state?.loading ? (
            <X size={17} strokeWidth={2} />
          ) : (
            <RotateCw size={16} strokeWidth={2} />
          )}
        </button>
        <button
          type="button"
          aria-label="主页"
          title="返回此资源网站主页"
          disabled={!state || busy}
          onClick={() => void act('home')}
        >
          <House size={17} strokeWidth={2} />
        </button>
      </nav>
      <form className="resource-browser-address" onSubmit={submit}>
        <span
          className="resource-browser-indicator"
          aria-label={state?.loading ? '正在加载' : '就绪'}
        >
          {state?.loading ? (
            <span className="resource-browser-spinner" />
          ) : (
            <span className="resource-browser-dot" />
          )}
        </span>
        <input
          ref={address}
          type="text"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onFocus={(event) => {
            editing.current = true
            event.currentTarget.select()
          }}
          onBlur={() => {
            editing.current = false
            setDraft(stateRef.current?.url ?? '')
          }}
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.stopPropagation()
              setDraft(stateRef.current?.url ?? '')
              address.current?.blur()
            }
          }}
          aria-label="网站地址"
          title={state?.url || '输入网站地址，按 Enter 打开'}
          placeholder="输入网站地址，按 Enter 打开"
          spellCheck={false}
          autoComplete="off"
        />
      </form>
      {error && (
        <div className="resource-browser-error" role="alert" title={error}>
          <span>{error}</span>
          {!state && (
            <button type="button" onClick={() => void readState()}>
              重试
            </button>
          )}
        </div>
      )}
    </main>
  )
}
