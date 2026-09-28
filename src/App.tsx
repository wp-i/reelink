import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type DragEvent,
  type ReactNode,
} from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { open } from '@tauri-apps/plugin-dialog'
import {
  ArrowRight,
  Check,
  ClipboardPaste,
  Copy,
  FileVideo,
  Folder,
  ImagePlus,
  Keyboard,
  LoaderCircle,
  RotateCcw,
  ScanLine,
  Search,
  X,
} from 'lucide-react'
import type {
  HistorySummary,
  PreviewRow,
  QrResult,
  RenameEdit,
  RenameItem,
  TmdbMatch,
} from './types'
import './styles.css'
import ResourceSites from './ResourceSites'

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown
  }
}
interface Preferences {
  shortcut: string
  shortcutAvailable: boolean
}
const desktop = Boolean(window.__TAURI_INTERNALS__)
const imageExtensions = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp']
const movieExtensions = ['mp4', 'mkv', 'avi', 'mov', 'wmv', 'm4v', 'ts', 'flv']
const errorText = (error: unknown) =>
  typeof error === 'string'
    ? error
    : error instanceof Error
      ? error.message
      : '操作未完成，请重试。'
function safeTitle(value: string) {
  return value
    .replace(/[<>:"/\\|?*\x00-\x1f]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/[. ]+$/, '')
}
function cleanTmdbName(match: TmdbMatch, row: PreviewRow) {
  const title = safeTitle(match.title)
  if (row.episode) return `${title} - ${row.episode}${row.extension}`
  if (!row.isDirectory && row.kind === 'tv') return row.targetName
  return `${title}${match.year ? ` (${match.year})` : ''}${row.isDirectory ? '' : row.extension}`
}

function ToolEmptyState({
  icon,
  title,
  description,
  actions,
}: {
  icon: ReactNode
  title: string
  description: string
  actions: ReactNode
}) {
  return (
    <div className="empty">
      <div className="tool-card">
        <div className="empty-symbol">{icon}</div>
        <p>{title}</p>
        <small>{description}</small>
        {actions}
      </div>
    </div>
  )
}

export default function App() {
  const [tab, setTab] = useState<'qr' | 'rename'>('qr')
  const [shortcut, setShortcut] = useState('Alt+Q')
  const [shortcutDraft, setShortcutDraft] = useState<string | null>(null)
  const [prefsBusy, setPrefsBusy] = useState(false)
  const [prefsError, setPrefsError] = useState('')
  const prefsBusyRef = useRef(false)
  const [qr, setQr] = useState<QrResult | null>(null)
  const [qrBusy, setQrBusy] = useState(false)
  const [renameBusy, setRenameBusy] = useState(false)
  const [rows, setRows] = useState<PreviewRow[]>([])
  const [history, setHistory] = useState<HistorySummary | null>(null)
  const [shortcutAvailable, setShortcutAvailable] = useState(true)
  const [notice, setNotice] = useState<{ tone: 'error' | 'success' | 'info'; text: string } | null>(
    null,
  )
  const [dropActive, setDropActive] = useState(false)
  const [tmdbRow, setTmdbRow] = useState<string | null>(null)
  const [tmdbQuery, setTmdbQuery] = useState('')
  const [tmdbMatches, setTmdbMatches] = useState<TmdbMatch[]>([])
  const [tmdbBusy, setTmdbBusy] = useState(false)
  const imageInput = useRef<HTMLInputElement>(null)
  const qrBusyRef = useRef(false)
  const renameBusyRef = useRef(false)
  const tabRef = useRef(tab)
  tabRef.current = tab
  const showError = useCallback(
    (error: unknown) => setNotice({ tone: 'error', text: errorText(error) }),
    [],
  )

  const runQr = useCallback(
    async (task: () => Promise<QrResult>) => {
      if (qrBusyRef.current) return
      qrBusyRef.current = true
      setQrBusy(true)
      setNotice(null)
      try {
        const result = await task()
        setQr(result)
        if (result.texts.length === 0)
          setNotice({ tone: 'info', text: '没有识别到二维码，请换一张清晰的图片。' })
      } catch (error) {
        showError(error)
      } finally {
        qrBusyRef.current = false
        setQrBusy(false)
      }
    },
    [showError],
  )

  const decodeFile = useCallback(
    (file: File) => {
      if (
        !file.type.startsWith('image/') &&
        !imageExtensions.some((ext) => file.name.toLowerCase().endsWith(`.${ext}`))
      ) {
        showError('请选择图片文件。')
        return
      }
      if (!desktop) {
        showError('本地图片识别需要桌面应用。')
        return
      }
      if (file.size > 24 * 1024 * 1024) {
        showError('图片超过 24 MB，请选择较小的图片。')
        return
      }
      void runQr(async () =>
        invoke<QrResult>('decode_image', {
          data: Array.from(new Uint8Array(await file.arrayBuffer())),
        }),
      )
    },
    [runQr, showError],
  )

  const previewPaths = useCallback(
    async (paths: string[]) => {
      if (!desktop || paths.length === 0 || renameBusyRef.current) return
      renameBusyRef.current = true
      setRenameBusy(true)
      setNotice(null)
      try {
        const items = await invoke<RenameItem[]>('preview_renames', { paths })
        setRows(items.map((item) => ({ ...item, selected: true, targetName: item.suggestedName })))
        setTmdbRow(null)
        if (items.length === 0) setNotice({ tone: 'info', text: '没有找到可整理的视频文件。' })
      } catch (error) {
        showError(error)
      } finally {
        renameBusyRef.current = false
        setRenameBusy(false)
      }
    },
    [showError],
  )

  const pickRename = async (directory: boolean) => {
    if (!desktop) {
      showError('选择本地文件需要桌面应用。')
      return
    }
    if (renameBusyRef.current) return
    try {
      const selected = await open(
        directory
          ? { multiple: true, directory: true, title: '选择文件夹' }
          : {
              multiple: true,
              directory: false,
              title: '选择视频文件',
              filters: [{ name: '视频文件', extensions: movieExtensions }],
            },
      )
      if (selected) await previewPaths(Array.isArray(selected) ? selected : [selected])
    } catch (error) {
      showError(error)
    }
  }

  useEffect(() => {
    if (!desktop) return
    void invoke<HistorySummary | null>('rename_history')
      .then(setHistory)
      .catch(() => {})
    void invoke<Preferences>('get_preferences')
      .then((prefs) => {
        setShortcut(prefs.shortcut)
        setShortcutAvailable(prefs.shortcutAvailable)
      })
      .catch((error) => {
        setShortcutAvailable(false)
        showError(error)
      })
  }, [])
  useEffect(() => {
    if (!desktop) return
    let active = true
    const cleanups: Array<() => void> = []
    void listen('capture-requested', () => {
      setTab('qr')
      void runQr(() => invoke<QrResult>('capture_qr'))
    })
      .then((unlisten) => {
        if (active) cleanups.push(unlisten)
        else unlisten()
      })
      .catch(showError)
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (document.querySelector('[data-resource-popover]')) return
        if (event.payload.type === 'enter' || event.payload.type === 'over') setDropActive(true)
        if (event.payload.type === 'leave') setDropActive(false)
        if (event.payload.type !== 'drop') return
        setDropActive(false)
        const paths = event.payload.paths
        if (tabRef.current === 'qr') {
          const path = paths.find((value) =>
            imageExtensions.some((ext) => value.toLowerCase().endsWith(`.${ext}`)),
          )
          if (path) void runQr(() => invoke<QrResult>('decode_image_path', { path }))
          else showError('请拖入图片文件。')
        } else if (tabRef.current === 'rename') void previewPaths(paths)
      })
      .then((unlisten) => {
        if (active) cleanups.push(unlisten)
        else unlisten()
      })
      .catch(showError)
    return () => {
      active = false
      cleanups.forEach((fn) => fn())
    }
  }, [previewPaths, runQr, showError])
  useEffect(() => {
    const onPaste = (event: ClipboardEvent) => {
      if (tabRef.current !== 'qr') return
      if (
        event.target instanceof HTMLElement &&
        event.target.closest('input, textarea, [contenteditable="true"]')
      )
        return
      const image = Array.from(event.clipboardData?.files ?? []).find((file) =>
        file.type.startsWith('image/'),
      )
      if (image) {
        event.preventDefault()
        decodeFile(image)
      }
    }
    window.addEventListener('paste', onPaste)
    return () => window.removeEventListener('paste', onPaste)
  }, [decodeFile])
  const saveShortcut = async () => {
    if (prefsBusyRef.current || !desktop || !shortcutDraft) return
    prefsBusyRef.current = true
    setPrefsBusy(true)
    setPrefsError('')
    try {
      const prefs = await invoke<Preferences>('update_preferences', { shortcut: shortcutDraft })
      setShortcut(prefs.shortcut)
      setShortcutAvailable(prefs.shortcutAvailable)
      setShortcutDraft(null)
    } catch (error) {
      setPrefsError(errorText(error))
    } finally {
      prefsBusyRef.current = false
      setPrefsBusy(false)
    }
  }
  const copyText = async (text: string) => {
    try {
      if (desktop) await invoke('copy_text', { text })
      else await navigator.clipboard.writeText(text)
      setNotice({ tone: 'success', text: '已复制到剪贴板。' })
    } catch (error) {
      showError(error)
    }
  }
  const refreshHistory = async () => {
    try {
      setHistory(await invoke<HistorySummary | null>('rename_history'))
    } catch {
      /* Keep the operation result visible. */
    }
  }
  const applyRenames = async () => {
    if (renameBusyRef.current) return
    const items: RenameEdit[] = rows
      .filter((row) => row.selected && row.targetName.trim() !== row.originalName)
      .map((row) => ({ id: row.id, sourcePath: row.sourcePath, targetName: row.targetName.trim() }))
    if (items.length === 0) {
      setNotice({ tone: 'info', text: '没有需要更名的已选项目。' })
      return
    }
    renameBusyRef.current = true
    setRenameBusy(true)
    setNotice(null)
    try {
      const outcome = await invoke<{ count: number; message: string }>('apply_renames', { items })
      setRows([])
      setNotice({ tone: 'success', text: outcome.message || `已更名 ${outcome.count} 项。` })
    } catch (error) {
      showError(error)
    } finally {
      await refreshHistory()
      renameBusyRef.current = false
      setRenameBusy(false)
    }
  }
  const undoRename = async () => {
    if (!desktop || !history || renameBusyRef.current) return
    renameBusyRef.current = true
    setRenameBusy(true)
    setNotice(null)
    try {
      const outcome = await invoke<{ count: number; message: string }>('undo_rename')
      setRows([])
      setNotice({ tone: 'success', text: outcome.message || `已撤销 ${outcome.count} 项更名。` })
    } catch (error) {
      showError(error)
    } finally {
      await refreshHistory()
      renameBusyRef.current = false
      setRenameBusy(false)
    }
  }
  const searchTmdb = async (row: PreviewRow) => {
    if (tmdbBusy) return
    if (!desktop) {
      showError('在线搜索需要桌面应用。')
      return
    }
    const query = tmdbQuery.trim()
    if (!query) {
      setNotice({ tone: 'info', text: '请输入片名。' })
      return
    }
    setTmdbBusy(true)
    setTmdbMatches([])
    setNotice(null)
    try {
      const matches = await invoke<TmdbMatch[]>('search_tmdb', { query, kind: row.kind })
      setTmdbMatches(matches)
      if (matches.length === 0) setNotice({ tone: 'info', text: '没有找到匹配作品，请换个名称。' })
    } catch (error) {
      showError(error)
    } finally {
      setTmdbBusy(false)
    }
  }
  const selectedCount = rows.filter(
    (row) => row.selected && row.targetName.trim() !== row.originalName,
  ).length

  const activeSearch = rows.find((row) => row.id === tmdbRow)
  const changed = (row: PreviewRow) => row.targetName.trim() !== row.originalName
  const setTarget = (row: PreviewRow, value: string) => {
    setRows((current) =>
      current.map((item) => {
        if (item.id === row.id) return { ...item, targetName: value }
        if (row.isDirectory && row.groupId && item.groupId === row.groupId && item.episode) {
          const title = safeTitle(value.replace(/\s*\(\d{4}\)$/, ''))
          return { ...item, targetName: `${title} - ${item.episode}${item.extension}` }
        }
        return item
      }),
    )
  }
  const chooseMatch = (match: TmdbMatch, row: PreviewRow) => {
    setRows((current) =>
      current.map((item) =>
        item.id === row.id || (row.groupId && item.groupId === row.groupId)
          ? {
              ...item,
              targetName: cleanTmdbName(match, item),
              title: match.title,
              year: match.year,
              confidence: 'high',
              notes: ['来自 TMDb'],
            }
          : item,
      ),
    )
    setTmdbRow(null)
  }
  const folders = rows.filter((row) => row.isDirectory).length
  const files = rows.length - folders
  const actions = (
    <div className="toolbar-actions">
      {tab === 'qr' ? (
        <>
          <button
            className="primary"
            disabled={qrBusy || !desktop}
            onClick={() => void runQr(() => invoke<QrResult>('decode_clipboard'))}
          >
            {qrBusy ? <LoaderCircle size={14} className="spin" /> : <ClipboardPaste size={14} />}
            粘贴
          </button>
          <button
            className="tool-button"
            disabled={qrBusy || !desktop}
            onClick={() => imageInput.current?.click()}
          >
            <ImagePlus size={14} />
            打开图片…
          </button>
        </>
      ) : (
        <>
          <button
            className="primary"
            disabled={renameBusy || !desktop}
            onClick={() => void pickRename(false)}
          >
            <FileVideo size={14} />
            选择文件…
          </button>
          <button
            className="tool-button"
            disabled={renameBusy || !desktop}
            onClick={() => void pickRename(true)}
          >
            <Folder size={14} />
            选择文件夹…
          </button>
        </>
      )}
    </div>
  )
  const hasContent = tab === 'qr' ? Boolean(qr) : rows.length > 0

  return (
    <div
      className={`app ${dropActive ? 'drop-active' : ''}`}
      data-app-ready="true"
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e: DragEvent<HTMLDivElement>) => {
        e.preventDefault()
        setDropActive(false)
        if (document.querySelector('[data-resource-popover]')) return
        const file = e.dataTransfer.files[0]
        if (tab === 'qr' && file) decodeFile(file)
      }}
    >
      <header className="toolbar">
        <div className="toolbar-leading">
          <div className="segments" role="tablist" aria-label="工具">
            <button
              role="tab"
              aria-selected={tab === 'qr'}
              onClick={() => {
                setTab('qr')
                setNotice(null)
              }}
            >
              二维码
            </button>
            <button
              role="tab"
              aria-selected={tab === 'rename'}
              onClick={() => {
                setTab('rename')
                setNotice(null)
              }}
            >
              重命名
            </button>
          </div>
          <ResourceSites desktop={desktop} />
        </div>
      </header>
      <main className="workspace">
        {hasContent && <div className="content-actions">{actions}</div>}
        {notice && (
          <div
            className={`notice ${notice.tone}`}
            role={notice.tone === 'error' ? 'alert' : 'status'}
          >
            <span>{notice.text}</span>
            <button className="icon-button" aria-label="关闭提示" onClick={() => setNotice(null)}>
              <X size={13} />
            </button>
          </div>
        )}
        {tab === 'qr' ? (
          <section aria-label="二维码识别">
            {!qr ? (
              <ToolEmptyState
                icon={<ScanLine size={25} strokeWidth={1.5} />}
                title="粘贴一张二维码截图"
                description="Ctrl + V 粘贴 · 也可拖入图片"
                actions={actions}
              />
            ) : (
              <div className="qr-content">
                <div className="result-heading">
                  <span>识别到 {qr.texts.length} 条内容</span>
                  <button
                    className="text-button"
                    onClick={() => {
                      setQr(null)
                      setNotice(null)
                    }}
                  >
                    清除
                  </button>
                </div>
                {qr.texts.length ? (
                  <div className="qr-results">
                    {qr.texts.map((text, index) => (
                      <div className="qr-result" key={`${index}-${text}`}>
                        <span className="result-number">{index + 1}</span>
                        <p>{text}</p>
                        <button
                          className="tool-button"
                          aria-label={`复制第 ${index + 1} 条结果`}
                          onClick={() => void copyText(text)}
                        >
                          <Copy size={13} />
                          复制
                        </button>
                      </div>
                    ))}
                  </div>
                ) : (
                  <p className="no-result">没有找到二维码，请换一张清晰的图片。</p>
                )}
              </div>
            )}
            <input
              ref={imageInput}
              type="file"
              hidden
              accept="image/png,image/jpeg,image/gif,image/webp,image/bmp"
              onChange={(e: ChangeEvent<HTMLInputElement>) => {
                const file = e.target.files?.[0]
                if (file) decodeFile(file)
                e.target.value = ''
              }}
            />
            {shortcutDraft !== null && (
              <div className="shortcut-editor">
                <label htmlFor="shortcut-input">截图快捷键</label>
                <input
                  id="shortcut-input"
                  aria-label="录入快捷键"
                  readOnly
                  autoFocus
                  placeholder="按下组合键"
                  value={shortcutDraft.replaceAll('+', ' + ')}
                  onKeyDown={(e) => {
                    if (e.key === 'Escape') {
                      setShortcutDraft(null)
                      setPrefsError('')
                      return
                    }
                    if (e.key === 'Tab') return
                    e.preventDefault()
                    e.stopPropagation()
                    const key = e.code.startsWith('Key')
                      ? e.code.slice(3)
                      : e.code.startsWith('Digit')
                        ? e.code.slice(5)
                        : /^F([1-9]|1[0-2])$/.test(e.code)
                          ? e.code
                          : null
                    if (!key) return
                    if (!e.ctrlKey && !e.altKey) {
                      setPrefsError('请同时按住 Ctrl 或 Alt。')
                      return
                    }
                    setPrefsError('')
                    setShortcutDraft(
                      [
                        e.ctrlKey ? 'Ctrl' : '',
                        e.altKey ? 'Alt' : '',
                        e.shiftKey ? 'Shift' : '',
                        key,
                      ]
                        .filter(Boolean)
                        .join('+'),
                    )
                  }}
                />
                <button
                  className="tool-button"
                  disabled={prefsBusy}
                  onClick={() => {
                    setShortcutDraft(null)
                    setPrefsError('')
                  }}
                >
                  取消
                </button>
                <button
                  className="primary"
                  disabled={!shortcutDraft || prefsBusy}
                  onClick={() => void saveShortcut()}
                >
                  保存
                </button>
                {prefsError && (
                  <p className="preference-error" role="alert">
                    {prefsError}
                  </p>
                )}
              </div>
            )}
          </section>
        ) : (
          <section aria-label="影视重命名">
            {rows.length ? (
              <>
                <div className="list-summary">
                  <span>
                    {folders > 0 ? `${folders} 个文件夹 · ` : ''}
                    {files} 个视频
                  </span>
                  <button
                    className="text-button"
                    disabled={renameBusy}
                    onClick={() => {
                      setRows([])
                      setTmdbRow(null)
                    }}
                  >
                    清空
                  </button>
                </div>
                {activeSearch && (
                  <div className="search-panel">
                    <form
                      onSubmit={(e) => {
                        e.preventDefault()
                        void searchTmdb(activeSearch)
                      }}
                    >
                      <Search size={14} />
                      <input
                        aria-label="搜索片名"
                        autoFocus
                        value={tmdbQuery}
                        onChange={(e) => setTmdbQuery(e.target.value)}
                        placeholder="输入作品名称"
                      />
                      <button className="primary" disabled={tmdbBusy || !tmdbQuery.trim()}>
                        {tmdbBusy ? '搜索中…' : '搜索'}
                      </button>
                      <button
                        type="button"
                        className="icon-button"
                        aria-label="关闭搜索"
                        onClick={() => setTmdbRow(null)}
                      >
                        <X size={14} />
                      </button>
                    </form>
                    {tmdbMatches.length > 0 && (
                      <div className="search-results">
                        {tmdbMatches.map((match) => (
                          <button
                            className="search-result"
                            key={match.id}
                            onClick={() => chooseMatch(match, activeSearch)}
                          >
                            <span>
                              {match.title} {match.year && <small>({match.year})</small>}
                              {match.originalTitle !== match.title && (
                                <small> · {match.originalTitle}</small>
                              )}
                            </span>
                            <ArrowRight size={13} />
                          </button>
                        ))}
                      </div>
                    )}
                    <div className="tmdb-credit">
                      <img src="/tmdb-logo.svg" alt="TMDB" />
                      <span>资料来自 TMDB · 本产品未经 TMDB 认可或认证</span>
                    </div>
                  </div>
                )}
                <div className="file-table" role="table" aria-label="更名预览">
                  <div className="table-head" role="row">
                    <input
                      type="checkbox"
                      aria-label="全选"
                      checked={rows.every((row) => row.selected)}
                      disabled={renameBusy}
                      onChange={(e) =>
                        setRows((current) =>
                          current.map((row) => ({ ...row, selected: e.target.checked })),
                        )
                      }
                    />
                    <span role="columnheader">原名称</span>
                    <span role="columnheader">新名称</span>
                    <span />
                  </div>
                  <div className="table-body">
                    {rows.map((row) => (
                      <div
                        className={`file-row ${row.isDirectory ? 'folder-row' : ''} ${!row.selected ? 'deselected' : ''}`}
                        key={row.id}
                        role="row"
                      >
                        <input
                          type="checkbox"
                          aria-label={`选择 ${row.originalName}`}
                          checked={row.selected}
                          disabled={renameBusy}
                          onChange={(e) =>
                            setRows((current) =>
                              current.map((item) =>
                                item.id === row.id ? { ...item, selected: e.target.checked } : item,
                              ),
                            )
                          }
                        />
                        <div
                          className={`source-cell ${row.episode ? 'episode-cell' : ''}`}
                          role="cell"
                          title={row.sourcePath}
                        >
                          {row.isDirectory ? <Folder size={14} /> : <FileVideo size={13} />}
                          <span>{row.originalName}</span>
                        </div>
                        <div className={`target-cell ${changed(row) ? 'changed' : ''}`} role="cell">
                          <input
                            aria-label={`${row.originalName} 的新名称`}
                            value={row.targetName}
                            title={row.targetName}
                            disabled={renameBusy}
                            onChange={(e) => setTarget(row, e.target.value)}
                            spellCheck={false}
                          />
                        </div>
                        {!row.episode || row.isDirectory ? (
                          <button
                            className="icon-button lookup"
                            aria-label={`查找 ${row.originalName}`}
                            title="从 TMDb 查找标准名称"
                            disabled={tmdbBusy || renameBusy}
                            onClick={() => {
                              setTmdbRow(row.id)
                              setTmdbQuery(row.title)
                              setTmdbMatches([])
                            }}
                          >
                            <Search size={14} />
                          </button>
                        ) : (
                          <span />
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              </>
            ) : (
              <ToolEmptyState
                icon={<Folder size={25} strokeWidth={1.5} />}
                title="整理影视名称"
                description="拖入视频，或选择文件与文件夹"
                actions={actions}
              />
            )}
          </section>
        )}
      </main>
      <footer className={`statusbar ${tab === 'rename' ? 'rename-status' : ''}`}>
        {tab === 'qr' ? (
          <>
            <span>{desktop ? '本地识别' : '桌面应用中可用'}</span>
            <button
              className={`shortcut-button ${shortcutAvailable ? '' : 'unavailable'}`}
              disabled={!desktop}
              aria-label="修改截图快捷键"
              title="修改截图快捷键"
              onClick={() => {
                setShortcutDraft(shortcut)
                setPrefsError('')
              }}
            >
              <Keyboard size={13} />
              <span>{shortcutAvailable ? '截图' : '快捷键被占用'}</span>
              <kbd>{shortcut.replaceAll('+', ' + ')}</kbd>
            </button>
          </>
        ) : (
          <>
            <div className="footer-left">
              {history ? (
                <button
                  className="text-button"
                  disabled={renameBusy}
                  onClick={() => void undoRename()}
                >
                  <RotateCcw size={13} />
                  撤销上次更名
                </button>
              ) : (
                <span>{rows.length ? '仅修改名称，保留目录结构' : '支持拖入文件或文件夹'}</span>
              )}
            </div>
            {rows.length > 0 && (
              <>
                <span>{selectedCount} 项更名</span>
                <button
                  className="primary"
                  disabled={renameBusy || selectedCount === 0}
                  onClick={() => void applyRenames()}
                >
                  {renameBusy ? <LoaderCircle size={13} className="spin" /> : <Check size={13} />}
                  应用更名
                </button>
              </>
            )}
          </>
        )}
      </footer>
    </div>
  )
}
