import { readFileSync, writeFileSync, mkdirSync, readdirSync, rmSync, existsSync } from 'node:fs'
import { join, basename, resolve } from 'node:path'

const ROOT = resolve(__dirname, '..')
const REF_DEST = resolve(ROOT, 'src/content/docs/reference')
const REF_SRC = resolve(ROOT, '../docs/references')
const LANG_SRC = resolve(ROOT, '../docs/language')

if (!existsSync(REF_SRC)) {
    console.error(`${REF_SRC} doesn't exist`)
    process.exit(0)
}

rmSync(REF_DEST, { recursive: true, force: true })
mkdirSync(REF_DEST, { recursive: true })

function processText(content) {
    const titleMatch = content.match(/^#\s+(.+)$/m)
    const title = titleMatch ? titleMatch[1].trim() : basename(file, '.md').replace(/-/g, ' ')

    if (titleMatch) {
        content = content.replace(titleMatch[0], '').replace(/^\n+/, '')
    }
    content = `---
title: ${JSON.stringify(title)}
description: ${JSON.stringify(title + ' standard library reference')} 
---
${content}`
    return content
}

for (const file of readdirSync(REF_SRC).filter(f => f.endsWith('.md'))) {
    let content = readFileSync(join(REF_SRC, file), 'utf-8')
    content = processText(content)
    writeFileSync(join(REF_DEST, file), content)
    console.log(`Wrote ${file}`)
}

writeFileSync(
    join(REF_DEST, 'attributes.md'),
    processText(readFileSync(join(LANG_SRC, 'attributes.md'), 'utf-8')),
)
console.log('Wrote attributes')

console.log(`Done`)
