import { useEffect, useLayoutEffect, useRef, useState, type FormEvent } from 'react'
import { createPortal } from 'react-dom'
import { invoke } from '@tauri-apps/api/core'
import { ChevronDown, Link2, MoreHorizontal, Pencil, Plus, X } from 'lucide-react'
import './resource-sites.css'

interface ResourceSite {
  id: string
  name: string
  url: string
}
type Panel = { kind: 'edit'; site?: ResourceSite } | { kind: 'more' } | { kind: 'error' }

function domain(url: string) {
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return url
  }
}

export default function ResourceSites({ desktop }: { desktop: boolean }) {
  const [sites, setSites] = useState<ResourceSite[]>([])
  const [loading, setLoading] = useState(desktop)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [panel, setPanel] = useState<Panel | null>(null)
  const [url, setUrl] = useState('')
  const [deleting, setDeleting] = useState(false)
  const [position, setPosition] = useState({ left: 8, top: 38, maxHeight: 160, width: 300 })
  const anchor = useRef<HTMLButtonElement | null>(null)
  const popup = useRef<HTMLDivElement>(null)
  const input = useRef<HTMLInputElement>(null)
  const toolbar = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!desktop) return
    let cancelled = false
    invoke<ResourceSite[]>('list_resource_sites')
      .then((value) => {
        if (!cancelled) setSites(value)
      })
      .catch((reason) => {
        if (!cancelled) setError(String(reason))
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [desktop])

  function close(restoreFocus = true) {
    if (busy) return
    setPanel(null)
    if (restoreFocus)
      requestAnimationFrame(() => {
        const target = anchor.current
        if (target?.isConnected && !target.disabled) target.focus()
        else toolbar.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
      })
  }

  function show(next: Panel, button?: HTMLButtonElement) {
    if (busy) return
    if (button) anchor.current = button
    setPanel(next)
    setUrl(next.kind === 'edit' ? (next.site?.url ?? '') : '')
    setDeleting(false)
    if (next.kind === 'edit') setError('')
  }

  useLayoutEffect(() => {
    if (!panel) return
    function positionPopup() {
      const rect = anchor.current?.getBoundingClientRect()
      const width = Math.min(300, Math.max(0, window.innerWidth - 16))
      const top = Math.min(rect ? rect.bottom + 5 : 38, Math.max(8, window.innerHeight - 80))
      setPosition({
        left: Math.max(
          8,
          Math.min(rect?.right ? rect.right - width : 8, window.innerWidth - width - 8),
        ),
        top,
        width,
        maxHeight: Math.max(0, window.innerHeight - top - 8),
      })
    }
    positionPopup()
    window.addEventListener('resize', positionPopup)
    return () => window.removeEventListener('resize', positionPopup)
  }, [panel])

  useEffect(() => {
    if (!panel) return
    if (panel.kind === 'edit') input.current?.focus()
    else popup.current?.focus()
  }, [panel])

  useEffect(() => {
    if (deleting)
      popup.current?.querySelector<HTMLButtonElement>('.resource-sites-delete button')?.focus()
  }, [deleting])

  useEffect(() => {
    if (!panel) return
    function outside(event: PointerEvent) {
      if (
        !busy &&
        !popup.current?.contains(event.target as Node) &&
        !toolbar.current?.contains(event.target as Node)
      )
        close(false)
    }
    function keyboard(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        event.preventDefault()
        close()
      }
      if (event.key === 'Tab') {
        const controls = popup.current?.querySelectorAll<HTMLElement>(
          'button:not(:disabled), input:not(:disabled)',
        )
        if (!controls?.length) {
          event.preventDefault()
          return
        }
        const first = controls[0],
          last = controls[controls.length - 1]
        if (!popup.current?.contains(document.activeElement)) {
          event.preventDefault()
          ;(event.shiftKey ? last : first).focus()
          return
        }
        if (
          event.shiftKey &&
          (document.activeElement === first || document.activeElement === popup.current)
        ) {
          event.preventDefault()
          last.focus()
        } else if (
          !event.shiftKey &&
          (document.activeElement === last || document.activeElement === popup.current)
        ) {
          event.preventDefault()
          first.focus()
        }
      }
    }
    document.addEventListener('pointerdown', outside)
    document.addEventListener('keydown', keyboard)
    return () => {
      document.removeEventListener('pointerdown', outside)
      document.removeEventListener('keydown', keyboard)
    }
  }, [panel, busy])

  async function save(event: FormEvent) {
    event.preventDefault()
    if (busy || !desktop || panel?.kind !== 'edit') return
    setBusy(true)
    setError('')
    try {
      const trimmed = url.trim()
      const hasScheme = /^[a-z][a-z\d+.-]*:/i.test(trimmed)
      const isHostWithPort = /^(?:[a-z\d-]+\.)+[a-z\d-]+:\d+(?:[/?#]|$)/i.test(trimmed)
      const normalized = hasScheme && !isHostWithPort ? trimmed : `https://${trimmed}`
      const parsed = new URL(normalized)
      if (!['http:', 'https:'].includes(parsed.protocol))
        throw new Error('仅支持 HTTP 或 HTTPS 网站地址。')
      const existing = panel.site
      const name =
        existing && existing.name !== domain(existing.url)
          ? existing.name
          : domain(normalized).slice(0, 80)
      setSites(
        await invoke<ResourceSite[]>('save_resource_site', {
          id: existing?.id ?? null,
          name,
          url: normalized,
        }),
      )
      setPanel(null)
      requestAnimationFrame(() =>
        toolbar.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus(),
      )
    } catch (reason) {
      setError(reason instanceof TypeError ? '请输入有效的网站地址。' : String(reason))
    } finally {
      setBusy(false)
    }
  }

  async function remove(site: ResourceSite) {
    if (busy || !desktop) return
    setBusy(true)
    setError('')
    try {
      setSites(await invoke<ResourceSite[]>('delete_resource_site', { id: site.id }))
      setPanel(null)
      requestAnimationFrame(() =>
        toolbar.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus(),
      )
    } catch (reason) {
      setError(String(reason))
    } finally {
      setBusy(false)
    }
  }

  async function open(site: ResourceSite, button: HTMLButtonElement) {
    if (busy || !desktop) return
    if (toolbar.current?.contains(button)) anchor.current = button
    setBusy(true)
    setError('')
    try {
      await invoke('open_resource_site', { id: site.id })
      setPanel(null)
    } catch (reason) {
      setError(String(reason))
      setPanel({ kind: 'error' })
    } finally {
      setBusy(false)
    }
  }

  const disabled = !desktop || busy || loading
  return (
    <div className="resource-sites" ref={toolbar} aria-label="资源快捷入口">
      {[0, 1].map((index) => {
        const site = sites[index]
        return (
          <div className="resource-site-slot" key={index}>
            <button
              type="button"
              className="tool-button resource-site-shortcut"
              disabled={disabled}
              aria-label={site ? `打开 ${site.name}` : `添加资源${index + 1}`}
              title={
                !desktop
                  ? '请在桌面应用中添加和打开资源网站'
                  : site
                    ? `${site.name}\n${site.url}`
                    : `添加资源${index + 1}`
              }
              onClick={(event) =>
                site
                  ? void open(site, event.currentTarget)
                  : show({ kind: 'edit' }, event.currentTarget)
              }
            >
              {!site && <Plus className="resource-site-full" size={12} />}
              <span className="resource-site-full">{site?.name ?? `资源${index + 1}`}</span>
              <span className="resource-site-compact" aria-hidden="true">
                {site ? <Link2 size={11} /> : <Plus size={11} />}
                {index + 1}
              </span>
            </button>
            {site && (
              <button
                type="button"
                className="tool-button resource-site-edit"
                disabled={disabled}
                aria-label={`编辑 ${site.name}`}
                title={`编辑 ${site.name}`}
                onClick={(event) => show({ kind: 'edit', site }, event.currentTarget)}
              >
                <ChevronDown size={11} />
              </button>
            )}
          </div>
        )
      })}
      {(sites.length >= 2 || error) && (
        <button
          type="button"
          className="tool-button resource-sites-more"
          disabled={disabled}
          aria-label="更多资源网站"
          title="更多资源网站"
          onClick={(event) => show({ kind: 'more' }, event.currentTarget)}
        >
          <MoreHorizontal size={14} />
        </button>
      )}
      {panel &&
        createPortal(
          <div
            className="resource-sites-popover"
            data-resource-popover="true"
            onPaste={(event) => event.stopPropagation()}
            onDrop={(event) => {
              event.preventDefault()
              event.stopPropagation()
            }}
            onDragOver={(event) => {
              event.preventDefault()
              event.stopPropagation()
            }}
            ref={popup}
            role="dialog"
            aria-label={
              panel.kind === 'edit' ? (panel.site ? '编辑资源网站' : '添加资源网站') : '资源网站'
            }
            tabIndex={-1}
            style={position}
          >
            <div className="resource-sites-popover-heading">
              <strong>
                {panel.kind === 'edit'
                  ? panel.site
                    ? '编辑资源网站'
                    : '添加资源网站'
                  : '资源网站'}
              </strong>
              <button
                type="button"
                className="icon-button"
                disabled={busy}
                aria-label="关闭资源网站菜单"
                onClick={() => close()}
              >
                <X size={13} />
              </button>
            </div>
            {error && (
              <p className="resource-sites-error" role="alert">
                {error}
              </p>
            )}
            {panel.kind === 'edit' && (
              <form className="resource-sites-form" onSubmit={save}>
                <input
                  id="resource-site-url"
                  aria-label="网站地址"
                  ref={input}
                  value={url}
                  onChange={(event) => setUrl(event.target.value)}
                  maxLength={4096}
                  required
                  disabled={busy}
                  placeholder="example.com"
                  autoComplete="off"
                  spellCheck={false}
                />
                {deleting ? (
                  <div className="resource-sites-delete">
                    <span>删除此收藏并关闭网站窗口？</span>
                    <div className="resource-sites-actions">
                      <button
                        type="button"
                        className="tool-button"
                        disabled={busy}
                        onClick={() => setDeleting(false)}
                      >
                        取消
                      </button>
                      <button
                        type="button"
                        className="tool-button"
                        disabled={busy}
                        onClick={() => panel.site && void remove(panel.site)}
                      >
                        删除
                      </button>
                    </div>
                  </div>
                ) : (
                  <div className="resource-sites-actions">
                    {panel.site && (
                      <button
                        type="button"
                        className="tool-button resource-site-remove"
                        disabled={busy}
                        onClick={() => setDeleting(true)}
                      >
                        删除
                      </button>
                    )}
                    <button
                      type="button"
                      className="tool-button"
                      disabled={busy}
                      onClick={() => close()}
                    >
                      取消
                    </button>
                    <button type="submit" className="primary" disabled={busy}>
                      {busy ? '处理中…' : '保存'}
                    </button>
                  </div>
                )}
              </form>
            )}
            {panel.kind === 'more' && (
              <>
                <div className="resource-sites-extra">
                  {sites.slice(2).map((site) => (
                    <div className="resource-sites-extra-row" key={site.id}>
                      <button
                        type="button"
                        className="tool-button"
                        disabled={busy}
                        title={site.url}
                        onClick={(event) => void open(site, event.currentTarget)}
                      >
                        <span>{site.name}</span>
                      </button>
                      <button
                        type="button"
                        className="icon-button"
                        disabled={busy}
                        aria-label={`编辑 ${site.name}`}
                        onClick={() => show({ kind: 'edit', site })}
                      >
                        <Pencil size={12} />
                      </button>
                    </div>
                  ))}
                </div>
                <button
                  type="button"
                  className="tool-button"
                  disabled={busy}
                  onClick={() => show({ kind: 'edit' })}
                >
                  <Plus size={12} />
                  添加网站
                </button>
              </>
            )}
          </div>,
          document.body,
        )}
    </div>
  )
}
