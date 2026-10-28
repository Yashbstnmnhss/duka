// @ts-check
import { readFile } from 'node:fs/promises'
import { defineConfig } from 'astro/config'
import starlight from '@astrojs/starlight'

// https://astro.build/config
export default defineConfig({
    site: 'https://yashbstnmnhss.github.io',
    base: '/duka',
    integrations: [
        starlight({
            title: 'Duka',
            logo: {
                src: './src/assets/moelogo.svg',
            },
            editLink: {
                baseUrl: 'https://github.com/Yashbstnmnhss/duka/edit/master/website/',
            },
            components: {
                Hero: './src/components/Hero.astro',
            },
            defaultLocale: 'root',
            locales: {
                root: {
                    label: 'English',
                    lang: 'en',
                },
                'zh-cn': {
                    label: '简体中文',
                    lang: 'zh-CN',
                },
            },
            expressiveCode: {
                shiki: {
                    // duka's own grammar, see extension/syntaxes
                    langs: [
                        async () => ({
                            ...JSON.parse(
                                await readFile(
                                    new URL(
                                        '../extension/syntaxes/duka.tmLanguage.json',
                                        import.meta.url,
                                    ),
                                    'utf-8',
                                ),
                            ),
                            name: 'duka',
                        }),
                    ],
                },
            },
            social: [
                { icon: 'github', label: 'GitHub', href: 'https://github.com/Yashbstnmnhss/duka' },
            ],
            sidebar: [
                {
                    label: 'Get started',
                    items: [
                        { label: 'Installation', slug: 'start/installation' },
                        { label: 'Your first kao', slug: 'start/first-project' },
                        { label: 'REPL & CLI', slug: 'start/repl' },
                    ],
                },
                {
                    label: 'Language',
                    items: [
                        { label: 'Overview', slug: 'language' },
                        { label: 'Lexical', slug: 'language/lexical' },
                        { label: 'Statements', slug: 'language/statements' },
                        { label: 'Expressions', slug: 'language/expressions' },
                        { label: 'Functions', slug: 'language/functions' },
                        { label: 'Types', slug: 'language/types', badge: 'type' },
                        { label: 'Type context', slug: 'language/type-context', badge: 'type' },
                        { label: 'Objects', slug: 'language/objects' },
                        { label: 'Match', slug: 'language/match' },
                        { label: 'Macros & bang', slug: 'language/macros' },
                        { label: 'Modules', slug: 'language/modules' },
                        { label: 'Coroutines', slug: 'language/coroutines' },
                        { label: 'Errors', slug: 'language/errors' },
                        { label: 'Keywords', slug: 'language/keywords' },
                        { label: 'Attributes', slug: 'language/attributes' },
                        { label: 'Config', slug: 'language/config' },
                    ],
                },
                {
                    label: 'Standard library',
                    items: [{ autogenerate: { directory: 'reference' } }],
                },
                {
                    label: 'Guides',
                    items: [
                        { label: 'Coming from lua', slug: 'guides/migrate-from-lua' },
                        { label: 'User data in rust', slug: 'guides/userdata-in-rust' },
                        { label: 'UI protocol', slug: 'guides/ui-duip' },
                    ],
                },
                {
                    label: 'Rust API',
                    items: [
                        { label: 'Overview', slug: 'rust' },
                        { label: 'Run scripts', slug: 'rust/run' },
                        { label: 'Compile & check', slug: 'rust/compile' },
                        { label: 'Extend the runtime', slug: 'rust/extend' },
                    ],
                },
            ],
        }),
    ],
})
