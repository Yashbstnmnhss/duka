import { readFileSync, writeFileSync, mkdirSync, readdirSync, rmSync, existsSync } from 'node:fs'
import { join, basename, resolve } from 'node:path'

const ROOT = process.cwd()
const DEST = resolve(ROOT, 'src/content/docs/reference')
const SRC = resolve(ROOT, '../docs/references')

if (!existsSync(SRC)) {
    console.error(`${SRC} doesn't exist`)
    process.exit(0)
}

rmSync(DEST, { recursive: true, force: true })
mkdirSync(DEST, { recursive: true })

for (const file of readdirSync(SRC).filter(f => f.endsWith('.md'))) {
    let content = readFileSync(join(SRC, file), 'utf-8')

    const titleMatch = content.match(/^#\s+(.+)$/m)
    const title = titleMatch ? titleMatch[1].trim() : basename(file, '.md').replace(/-/g, ' ')

    if (titleMatch) {
        content = content.replace(titleMatch[0], '').replace(/^\n+/, '')
    }
    content = `---
title: ${JSON.stringify(title)}
description: ${JSON.stringify(title + 'standard library reference')} 
---
${content}`

    writeFileSync(join(DEST, file), content)
    console.log(`Wrote ${file}`)
}

console.log(`Done`)
