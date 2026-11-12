import { SESSION_TTL, type OperationResponse, type SelectionRect } from './shared'

interface SelectionStart {
  sessionId: string
  imageDataUrl: string
}

interface SelectionOptions {
  request: (
    type: 'decode' | 'copy' | 'cancel',
    rectOrIndex?: SelectionRect | number,
  ) => Promise<OperationResponse>
  onClose?: () => void
  fit?: 'viewport' | 'contain'
}

export interface SelectionHandle {
  close: (notify?: boolean) => void
  ready: Promise<void>
}

const styles = `
:host { all: initial; position: fixed; inset: 0; z-index: 2147483647; display: block; color-scheme: light; }
*, *::before, *::after { box-sizing: border-box; }
.root { position: fixed; inset: 0; overflow: hidden; background: #161b27; color: #1e2737; font: 14px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; user-select: none; touch-action: none; }
.shot { position: absolute; display: block; width: 100%; height: 100%; pointer-events: none; }
.shade { position: absolute; inset: 0; background: rgba(17,24,39,.30); cursor: crosshair; touch-action: none; }
.box { position: absolute; display: none; border: 2px solid #fff; border-radius: 7px; box-shadow: 0 0 0 1px rgba(27,37,58,.35), 0 0 0 9999px rgba(18,26,42,.27); pointer-events: none; }
.tip { position: absolute; left: 50%; top: 16px; transform: translateX(-50%); max-width: calc(100vw - 32px); padding: 9px 15px; border-radius: 16px; background: rgba(252,253,255,.92); box-shadow: 0 9px 30px rgba(16,25,47,.22); color: #26344a; font-size: 13px; text-align: center; overflow-wrap: anywhere; pointer-events: none; }
.panel { position: absolute; top: 20px; right: 20px; width: min(360px, calc(100vw - 32px)); max-height: min(460px, calc(100vh - 40px)); display: none; flex-direction: column; gap: 12px; padding: 16px; border: 1px solid rgba(255,255,255,.9); border-radius: 20px; background: rgba(250,251,254,.96); box-shadow: 0 18px 54px rgba(16,25,47,.27); backdrop-filter: blur(20px); -webkit-backdrop-filter: blur(20px); overflow: auto; touch-action: auto; user-select: text; }
.panel.show { display: flex; }
.heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.title { font-size: 15px; font-weight: 700; }
.status { color: #526078; font-size: 13px; overflow-wrap: anywhere; }
.result { display: flex; align-items: center; gap: 9px; min-width: 0; padding: 9px; border-radius: 13px; background: #eef1f6; }
.value { flex: 1; min-width: 0; max-height: 78px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; font-size: 12px; user-select: text; }
.actions { display: flex; gap: 8px; flex-wrap: wrap; }
button { appearance: none; border: 0; border-radius: 999px; background: #e8ecf3; color: #26344a; padding: 7px 12px; font-family: inherit; font-size: 12px; font-weight: 600; line-height: 1.4; cursor: pointer; white-space: nowrap; }
button.primary { background: #303d56; color: #fff; }
button:hover { filter: brightness(.96); }
button:focus-visible { outline: 2px solid #5782e5; outline-offset: 2px; }
button:disabled { opacity: .55; cursor: wait; }
`

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag)
  if (className) node.className = className
  if (text !== undefined) node.textContent = text
  return node
}

export function mountSelection(start: SelectionStart, options: SelectionOptions): SelectionHandle {
  const previousFocus =
    document.activeElement instanceof HTMLElement ? document.activeElement : null
  const host = el('div')
  const shadow = host.attachShadow({ mode: 'closed' })
  const sheet = new CSSStyleSheet()
  sheet.replaceSync(styles)
  shadow.adoptedStyleSheets = [sheet]
  const root = el('div', 'root')
  const shot = el('img', 'shot')
  shot.alt = ''
  shot.draggable = false
  const shade = el('div', 'shade')
  shade.tabIndex = -1
  const box = el('div', 'box')
  const tip = el('div', 'tip', '拖动框选二维码 · Esc 取消')
  const panel = el('section', 'panel')
  panel.setAttribute('role', 'dialog')
  panel.setAttribute('aria-label', '二维码识别结果')
  const heading = el('div', 'heading')
  const title = el('div', 'title', '识别结果')
  const closeButton = el('button', '', '关闭')
  const status = el('div', 'status')
  const results = el('div')
  const actions = el('div', 'actions')
  const retryButton = el('button', '', '重新框选')
  heading.append(title, closeButton)
  actions.append(retryButton)
  panel.append(heading, status, results, actions)
  root.append(shot, shade, box, tip, panel)
  shadow.append(root)
  document.documentElement.append(host)

  let closed = false
  let busy = false
  let resizeInvalid = false
  let anchor: { x: number; y: number } | null = null
  let area = { left: 0, top: 0, width: innerWidth, height: innerHeight }
  let timer = 0
  let readyTimer = 0
  let readySettled = false
  let resolveReady!: () => void
  let rejectReady!: (reason: Error) => void
  const ready = new Promise<void>((resolve, reject) => {
    resolveReady = resolve
    rejectReady = reject
  })

  const failReady = () => {
    if (readySettled) return
    close(false)
  }

  const close = (notify = true) => {
    if (closed) return
    closed = true
    clearTimeout(timer)
    clearTimeout(readyTimer)
    if (!readySettled) {
      readySettled = true
      rejectReady(new Error('截图无法在当前页面显示。'))
    }
    document.removeEventListener('keydown', onKeydown, true)
    window.removeEventListener('resize', onResize)
    shot.removeAttribute('src')
    host.remove()
    if (notify) void options.request('cancel').catch(() => undefined)
    if (previousFocus?.isConnected) previousFocus.focus({ preventScroll: true })
    options.onClose?.()
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopImmediatePropagation()
      close()
      return
    }
    if (event.key === 'Tab' && panel.classList.contains('show')) {
      const focusable = Array.from(
        panel.querySelectorAll<HTMLButtonElement>('button:not([disabled])'),
      )
      if (!focusable.length) return
      const active = shadow.activeElement
      const index = focusable.findIndex((button) => button === active)
      if (index === -1) {
        event.preventDefault()
        focusable[event.shiftKey ? focusable.length - 1 : 0].focus()
      } else if (!event.shiftKey && index === focusable.length - 1) {
        event.preventDefault()
        focusable[0].focus()
      } else if (event.shiftKey && index === 0) {
        event.preventDefault()
        focusable[focusable.length - 1].focus()
      }
    }
  }

  function onResize() {
    if (options.fit === 'viewport') {
      if (resizeInvalid || (innerWidth === area.width && innerHeight === area.height)) return
      resizeInvalid = true
      anchor = null
      results.replaceChildren()
      box.style.display = 'none'
      shot.removeAttribute('src')
      retryButton.disabled = true
      shade.style.cursor = 'default'
      showStatus('页面尺寸已变化，截图已取消。请关闭后重新截图。')
      void options.request('cancel').catch(() => undefined)
    } else {
      anchor = null
      box.style.display = 'none'
      layoutImage()
    }
  }

  function layoutImage() {
    if (options.fit !== 'contain') {
      area = { left: 0, top: 0, width: innerWidth, height: innerHeight }
      shot.style.left = '0'
      shot.style.top = '0'
      shot.style.width = '100%'
      shot.style.height = '100%'
      return
    }
    const iw = shot.naturalWidth || innerWidth
    const ih = shot.naturalHeight || innerHeight
    const scale = Math.min(innerWidth / iw, innerHeight / ih)
    area = {
      left: (innerWidth - iw * scale) / 2,
      top: (innerHeight - ih * scale) / 2,
      width: iw * scale,
      height: ih * scale,
    }
    Object.assign(shot.style, {
      left: `${area.left}px`,
      top: `${area.top}px`,
      width: `${area.width}px`,
      height: `${area.height}px`,
    })
  }

  const point = (event: PointerEvent) => ({
    x: Math.max(area.left, Math.min(area.left + area.width, event.clientX)),
    y: Math.max(area.top, Math.min(area.top + area.height, event.clientY)),
  })

  function draw(from: { x: number; y: number }, to: { x: number; y: number }) {
    const left = Math.min(from.x, to.x)
    const top = Math.min(from.y, to.y)
    Object.assign(box.style, {
      display: 'block',
      left: `${left}px`,
      top: `${top}px`,
      width: `${Math.abs(to.x - from.x)}px`,
      height: `${Math.abs(to.y - from.y)}px`,
    })
  }

  function showStatus(message: string) {
    status.textContent = message
    panel.classList.add('show')
    tip.style.display = 'none'
    closeButton.focus({ preventScroll: true })
  }

  shade.addEventListener('pointerdown', (event) => {
    if (busy || resizeInvalid || event.button !== 0) return
    const p = point(event)
    if (event.clientX !== p.x || event.clientY !== p.y) return
    anchor = p
    shade.setPointerCapture(event.pointerId)
    draw(p, p)
    panel.classList.remove('show')
    tip.style.display = 'none'
    shade.focus({ preventScroll: true })
  })
  shade.addEventListener('pointermove', (event) => {
    if (anchor) draw(anchor, point(event))
  })
  shade.addEventListener('pointerup', async (event) => {
    if (!anchor || busy || resizeInvalid) return
    const end = point(event)
    const from = anchor
    anchor = null
    if (shade.hasPointerCapture(event.pointerId)) shade.releasePointerCapture(event.pointerId)
    const width = Math.abs(end.x - from.x)
    const height = Math.abs(end.y - from.y)
    if (width < 12 || height < 12) {
      box.style.display = 'none'
      showStatus('选区太小，请重新框选二维码。')
      return
    }
    busy = true
    showStatus('正在本地识别…')
    const rect: SelectionRect = {
      x: Math.min(from.x, end.x) - area.left,
      y: Math.min(from.y, end.y) - area.top,
      width,
      height,
      viewportWidth: area.width,
      viewportHeight: area.height,
    }
    try {
      const response = await options.request('decode', rect)
      if (closed || resizeInvalid) return
      if (!response.ok) {
        showStatus(response.error || '识别失败，请重新框选。')
        return
      }
      const texts = response.texts || []
      results.replaceChildren()
      if (!texts.length) {
        showStatus('选区内没有识别到二维码，请重新框选。')
        return
      }
      showStatus(
        texts.length === 1
          ? response.copied === true
            ? '已复制到剪贴板'
            : '识别成功，请点击复制'
          : `识别到 ${texts.length} 个二维码，请选择复制`,
      )
      texts.forEach((value, index) => {
        const row = el('div', 'result')
        const text = el('div', 'value', value)
        row.append(text)
        const copy = el('button', 'primary', '复制')
        copy.addEventListener('click', async () => {
          copy.disabled = true
          try {
            const answer = await options.request('copy', index)
            if (!closed && !resizeInvalid)
              showStatus(
                answer.ok && answer.copied === true
                  ? '已复制到剪贴板'
                  : answer.error || '复制失败，请重试。',
              )
          } catch {
            if (!closed && !resizeInvalid) showStatus('复制失败，请重试。')
          } finally {
            copy.disabled = false
          }
        })
        row.append(copy)
        results.append(row)
      })
    } catch {
      if (!closed && !resizeInvalid) showStatus('识别失败，请重新框选。')
    } finally {
      busy = false
    }
  })
  shade.addEventListener('pointercancel', () => {
    anchor = null
    box.style.display = 'none'
  })
  closeButton.addEventListener('click', () => close())
  retryButton.addEventListener('click', () => {
    if (busy || resizeInvalid) return
    results.replaceChildren()
    status.textContent = ''
    panel.classList.remove('show')
    box.style.display = 'none'
    tip.style.display = ''
  })
  document.addEventListener('keydown', onKeydown, true)
  window.addEventListener('resize', onResize)
  shot.addEventListener('load', layoutImage, { once: true })
  shot.addEventListener(
    'load',
    () => {
      requestAnimationFrame(() => {
        if (closed || readySettled) return
        if (
          shot.naturalWidth < 1 ||
          shot.naturalHeight < 1 ||
          host.getClientRects().length === 0 ||
          getComputedStyle(host).visibility !== 'visible' ||
          Number(getComputedStyle(host).opacity) === 0 ||
          getComputedStyle(root).position !== 'fixed' ||
          getComputedStyle(shade).cursor !== 'crosshair'
        ) {
          failReady()
          return
        }
        readySettled = true
        clearTimeout(readyTimer)
        resolveReady()
      })
    },
    { once: true },
  )
  shot.addEventListener('error', failReady, { once: true })
  readyTimer = window.setTimeout(failReady, 5000)
  shot.src = start.imageDataUrl
  layoutImage()
  timer = window.setTimeout(() => {
    close()
  }, SESSION_TTL)
  return { close, ready }
}
