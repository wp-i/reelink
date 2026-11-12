import assert from 'node:assert/strict'
import test from 'node:test'
import QRCode from 'qrcode/lib/core/qrcode.js'
import { decodePixels } from './decode'

interface Image {
  data: Uint8ClampedArray
  width: number
  height: number
}

function blank(width: number, height: number): Image {
  const data = new Uint8ClampedArray(width * height * 4)
  data.fill(255)
  return { data, width, height }
}

function paint(image: Image, value: string, left: number, top: number, inverted = false): void {
  const qr = QRCode.create(value, { errorCorrectionLevel: 'M' })
  const cell = 5
  const quiet = 4
  const size = qr.modules.size
  for (let row = -quiet; row < size + quiet; row++) {
    for (let col = -quiet; col < size + quiet; col++) {
      const dark =
        row >= 0 && row < size && col >= 0 && col < size && Boolean(qr.modules.get(row, col))
      const gray = dark !== inverted ? 0 : 255
      for (let dy = 0; dy < cell; dy++) {
        for (let dx = 0; dx < cell; dx++) {
          const x = left + (col + quiet) * cell + dx
          const y = top + (row + quiet) * cell + dy
          const offset = (y * image.width + x) * 4
          image.data[offset] = image.data[offset + 1] = image.data[offset + 2] = gray
          image.data[offset + 3] = 255
        }
      }
    }
  }
}

test('decodes one UTF-8 QR without modifying source pixels', () => {
  const image = blank(240, 240)
  paint(image, '本地识别 🔗', 30, 30)
  const original = new Uint8ClampedArray(image.data)
  assert.deepEqual(decodePixels(image.data, image.width, image.height), ['本地识别 🔗'])
  assert.deepEqual(image.data, original)
})

test('decodes multiple distinct QRs, including inverted, while deduplicating payloads', () => {
  const image = blank(720, 240)
  paint(image, 'https://example.org/a', 20, 20)
  paint(image, 'https://example.org/b', 260, 20, true)
  paint(image, 'https://example.org/a', 500, 20)
  assert.deepEqual(
    new Set(decodePixels(image.data, image.width, image.height)),
    new Set(['https://example.org/a', 'https://example.org/b']),
  )
})

test('keeps a nearby symbol intact when masking a decoded symbol', () => {
  const image = blank(410, 200)
  paint(image, 'left', 10, 10)
  paint(image, 'right', 175, 10)
  assert.deepEqual(
    new Set(decodePixels(image.data, image.width, image.height)),
    new Set(['left', 'right']),
  )
})

test('returns an empty result for blank pixels', () => {
  const image = blank(120, 120)
  assert.deepEqual(decodePixels(image.data, image.width, image.height), [])
})

test('rejects invalid or excessive image dimensions before scanning', () => {
  assert.throws(() => decodePixels(new Uint8ClampedArray(4), NaN, 1))
  assert.throws(() => decodePixels(new Uint8ClampedArray(4), -1, 1))
  assert.throws(() => decodePixels(new Uint8ClampedArray(4), 5000, 5000), /2000 万/)
  assert.throws(() => decodePixels(new Uint8ClampedArray(4), 2, 2))
})
