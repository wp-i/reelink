import assert from 'node:assert/strict'
import test from 'node:test'
import { cropRect } from './geometry'

test('uses captured image / viewport scale, including non-integer high DPI scaling', () => {
  assert.deepEqual(
    cropRect(
      {
        x: 10.5,
        y: 20.25,
        width: 100.25,
        height: 60.5,
        viewportWidth: 800,
        viewportHeight: 600,
      },
      1500,
      1125,
    ),
    { x: 19, y: 37, width: 189, height: 115 },
  )
})

test('clamps a selection ending at the viewport edge', () => {
  assert.deepEqual(
    cropRect(
      {
        x: 400,
        y: 100,
        width: 400,
        height: 500,
        viewportWidth: 800,
        viewportHeight: 600,
      },
      1600,
      1200,
    ),
    { x: 800, y: 200, width: 800, height: 1000 },
  )
})

test('rejects malformed, tiny, and out-of-viewport selections', () => {
  const base = { x: 10, y: 10, width: 20, height: 20, viewportWidth: 100, viewportHeight: 100 }
  for (const patch of [
    { x: NaN },
    { y: Infinity },
    { width: 3.99 },
    { height: 0 },
    { width: -20 },
    { viewportWidth: 0 },
    { x: -1 },
    { x: 90 },
    { y: 90 },
  ]) {
    assert.throws(() => cropRect({ ...base, ...patch }, 200, 200))
  }
  assert.throws(() => cropRect(base, NaN, 200))
  assert.throws(() => cropRect(base, 200.5, 200))
})
