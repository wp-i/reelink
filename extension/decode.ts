import jsQR from 'jsqr'

const MAX_PIXELS = 20_000_000
const MAX_CODES = 16
const MAX_SCANS = 48

interface Point {
  x: number
  y: number
}

function insidePolygon(x: number, y: number, points: Point[]): boolean {
  let inside = false
  for (let i = 0, j = points.length - 1; i < points.length; j = i++) {
    const a = points[i]
    const b = points[j]
    if (a.y > y !== b.y > y && x < ((b.x - a.x) * (y - a.y)) / (b.y - a.y) + a.x) {
      inside = !inside
    }
  }
  return inside
}

function maskCode(
  data: Uint8ClampedArray,
  width: number,
  height: number,
  corners: Point[],
): boolean {
  if (corners.some(({ x, y }) => !Number.isFinite(x) || !Number.isFinite(y))) return false
  const left = Math.max(0, Math.floor(Math.min(...corners.map(({ x }) => x))))
  const top = Math.max(0, Math.floor(Math.min(...corners.map(({ y }) => y))))
  const right = Math.min(width, Math.ceil(Math.max(...corners.map(({ x }) => x))))
  const bottom = Math.min(height, Math.ceil(Math.max(...corners.map(({ y }) => y))))
  if (right <= left || bottom <= top) return false

  let changed = false
  for (let y = top; y < bottom; y++) {
    for (let x = left; x < right; x++) {
      if (!insidePolygon(x + 0.5, y + 0.5, corners)) continue
      const offset = (y * width + x) * 4
      if (data[offset] !== 255 || data[offset + 1] !== 255 || data[offset + 2] !== 255) {
        changed = true
      }
      data[offset] = data[offset + 1] = data[offset + 2] = data[offset + 3] = 255
    }
  }
  return changed
}

/** Decode up to 16 distinct QR payloads from an in-memory RGBA image. */
export function decodePixels(data: Uint8ClampedArray, width: number, height: number): string[] {
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width <= 0 || height <= 0) {
    throw new Error('图片尺寸无效')
  }
  const pixels = width * height
  if (pixels > MAX_PIXELS) throw new Error('图片过大，最多支持 2000 万像素')
  if (!(data instanceof Uint8ClampedArray) || data.length !== pixels * 4) {
    throw new Error('图片像素数据无效')
  }

  const working = new Uint8ClampedArray(data)
  const found: string[] = []
  const seen = new Set<string>()
  let scans = 0

  function scanRegion(x: number, y: number, regionWidth: number, regionHeight: number): void {
    const region =
      x === 0 && y === 0 && regionWidth === width && regionHeight === height
        ? working
        : new Uint8ClampedArray(regionWidth * regionHeight * 4)
    if (region !== working) {
      for (let row = 0; row < regionHeight; row++) {
        const start = ((y + row) * width + x) * 4
        region.set(working.subarray(start, start + regionWidth * 4), row * regionWidth * 4)
      }
    }
    // jsQR 1.4's `onlyInvert` path does not initialize its inverted bitmap.
    for (const inversionAttempts of ['dontInvert', 'invertFirst'] as const) {
      while (scans < MAX_SCANS && found.length < MAX_CODES) {
        scans++
        const code = jsQR(region, regionWidth, regionHeight, { inversionAttempts })
        if (!code) break
        const { topLeftCorner, topRightCorner, bottomRightCorner, bottomLeftCorner } = code.location
        const corners = [topLeftCorner, topRightCorner, bottomRightCorner, bottomLeftCorner]
        const masked = maskCode(region, regionWidth, regionHeight, corners)
        if (region !== working) {
          maskCode(
            working,
            width,
            height,
            corners.map((point) => ({ x: point.x + x, y: point.y + y })),
          )
        }
        // A corrupt location must not cause the same QR to be decoded indefinitely.
        if (!masked) break
        if (code.data.length > 0 && !seen.has(code.data)) {
          seen.add(code.data)
          found.push(code.data)
        }
      }
    }
  }

  scanRegion(0, 0, width, height)
  // Overlapping windows isolate neighboring symbols that confuse jsQR's
  // full-image locator. Keep the other axis full-sized for wide/tall symbols.
  for (const fraction of [0.45, 0.6]) {
    const horizontalWidth = Math.ceil(width * fraction)
    if (horizontalWidth < width) {
      for (const x of [0, Math.floor((width - horizontalWidth) / 2), width - horizontalWidth]) {
        if (scans >= MAX_SCANS || found.length >= MAX_CODES) break
        scanRegion(x, 0, horizontalWidth, height)
      }
    }
  }
  for (const fraction of [0.45, 0.6]) {
    const verticalHeight = Math.ceil(height * fraction)
    if (verticalHeight < height) {
      for (const y of [0, Math.floor((height - verticalHeight) / 2), height - verticalHeight]) {
        if (scans >= MAX_SCANS || found.length >= MAX_CODES) break
        scanRegion(0, y, width, verticalHeight)
      }
    }
  }
  return found
}
