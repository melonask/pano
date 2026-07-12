import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'Pano',
  description: 'Multi-chain deposit detection',
  base: '/pano/',
  themeConfig: {
    nav: [
      { text: 'Guide', link: '/guide/getting-started' },
      { text: 'Reference', link: '/reference/operations' },
      { text: 'GitHub', link: 'https://github.com/melonask/pano' }
    ],
    sidebar: [
      {
        text: 'Guide',
        items: [
          { text: 'Getting started', link: '/guide/getting-started' },
          { text: 'Configuration', link: '/guide/configuration' },
          { text: 'Usage', link: '/guide/usage' }
        ]
      },
      {
        text: 'Reference',
        items: [
          { text: 'Responses', link: '/reference/responses' },
          { text: 'Operations', link: '/reference/operations' }
        ]
      }
    ]
  }
})
