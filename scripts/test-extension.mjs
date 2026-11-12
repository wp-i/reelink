import { build } from 'esbuild'
import { spawnSync } from 'node:child_process'
import { mkdirSync, readdirSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const output = join(root, 'artifacts', 'extension-tests')
mkdirSync(output, { recursive: true })
const files = readdirSync(join(root, 'extension')).filter((name) => name.endsWith('.test.ts'))
if (!files.length) throw new Error('No extension tests found')
await build({
  absWorkingDir: root,
  entryPoints: ['extension/background.ts'],
  outfile: join(output, 'background.vm.js'),
  bundle: true,
  format: 'iife',
  target: 'chrome116',
})
await build({
  absWorkingDir: root,
  entryPoints: files.map((file) => `extension/${file}`),
  outdir: output,
  bundle: true,
  platform: 'node',
  external: ['esbuild'],
  format: 'esm',
  target: 'node22',
  outExtension: { '.js': '.mjs' },
})
const result = spawnSync(
  process.execPath,
  ['--test', ...files.map((file) => join(output, file.replace(/\.ts$/, '.mjs')))],
  { stdio: 'inherit' },
)
process.exitCode = result.status || 0
