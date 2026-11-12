import type { SelectionRect } from './shared'

export interface PixelRect {
  x: number
  y: number
  width: number
  height: number
}

/** Map a CSS viewport selection onto the captured bitmap, including fractional scale factors. */
export function cropRect(rect: SelectionRect, imageWidth: number, imageHeight: number): PixelRect {
  const values = [
    rect.x,
    rect.y,
    rect.width,
    rect.height,
    rect.viewportWidth,
    rect.viewportHeight,
    imageWidth,
    imageHeight,
  ]
  if (values.some((value) => !Number.isFinite(value))) {
    throw new Error('选区或截图尺寸无效')
  }
  if (
    !Number.isSafeInteger(imageWidth) ||
    !Number.isSafeInteger(imageHeight) ||
    imageWidth <= 0 ||
    imageHeight <= 0 ||
    rect.viewportWidth <= 0 ||
    rect.viewportHeight <= 0 ||
    rect.x < 0 ||
    rect.y < 0 ||
    rect.width < 4 ||
    rect.height < 4 ||
    rect.x + rect.width > rect.viewportWidth ||
    rect.y + rect.height > rect.viewportHeight
  ) {
    throw new Error('选区超出截图范围或过小')
  }

  const x = Math.min(imageWidth, Math.floor((rect.x * imageWidth) / rect.viewportWidth))
  const y = Math.min(imageHeight, Math.floor((rect.y * imageHeight) / rect.viewportHeight))
  const right = Math.min(
    imageWidth,
    Math.ceil(((rect.x + rect.width) * imageWidth) / rect.viewportWidth),
  )
  const bottom = Math.min(
    imageHeight,
    Math.ceil(((rect.y + rect.height) * imageHeight) / rect.viewportHeight),
  )
  if (right <= x || bottom <= y) {
    throw new Error('选区在截图中没有可用像素')
  }
  return { x, y, width: right - x, height: bottom - y }
}
