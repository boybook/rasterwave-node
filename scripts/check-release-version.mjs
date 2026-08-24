import { execFileSync } from 'node:child_process'
import { readFileSync, readdirSync } from 'node:fs'
import path from 'node:path'

const tag = process.env.GITHUB_REF_NAME || process.argv[2]
if (!tag || !/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(tag)) {
  throw new Error(`Expected a vX.Y.Z release tag, received ${tag || '<empty>'}`)
}
const expected = tag.slice(1)
const root = JSON.parse(readFileSync('package.json', 'utf8'))
if (root.version !== expected) throw new Error(`package.json is ${root.version}, tag is ${expected}`)

const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--no-deps', '--format-version', '1'], { encoding: 'utf8' }))
const crate = metadata.packages.find(pkg => pkg.name === 'rasterwave-node')
if (!crate || crate.version !== expected) throw new Error(`Cargo.toml is ${crate?.version}, tag is ${expected}`)

for (const directory of readdirSync('npm')) {
  const manifest = JSON.parse(readFileSync(path.join('npm', directory, 'package.json'), 'utf8'))
  if (manifest.version !== expected) throw new Error(`${manifest.name} is ${manifest.version}, tag is ${expected}`)
}

console.log(`Release version ${expected} is consistent across Cargo and npm manifests`)
