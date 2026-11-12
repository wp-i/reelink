import type { BackgroundRequest, OperationResponse } from './shared'

const capture = document.getElementById('capture') as HTMLButtonElement
const shortcut = document.getElementById('shortcut')!
const status = document.getElementById('status')!
const settings = document.getElementById('settings') as HTMLButtonElement

void chrome.commands
  .getAll()
  .then((commands) => {
    const command =
      commands.find((item) => item.name === 'capture-qr') ||
      commands.find((item) => item.name !== '_execute_action')
    shortcut.textContent = command?.shortcut || '未设置'
  })
  .catch(() => {
    shortcut.textContent = '读取失败'
  })

capture.addEventListener('click', async () => {
  capture.disabled = true
  status.textContent = '正在截图…'
  try {
    const response = (await chrome.runtime.sendMessage({
      target: 'background',
      type: 'start',
    } satisfies BackgroundRequest)) as OperationResponse
    if (response.ok) window.close()
    else status.textContent = response.error || '无法截图，请确认浏览器处于前台。'
  } catch {
    status.textContent = '无法截图，请确认浏览器处于前台并重试。'
  } finally {
    capture.disabled = false
  }
})

settings.addEventListener('click', () => {
  void chrome.tabs.create({ url: 'chrome://extensions/shortcuts' })
})
