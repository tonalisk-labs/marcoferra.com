// @ts-check
import { defineConfig } from 'astro/config';

import react from '@astrojs/react';
import sitemap from '@astrojs/sitemap';
import tailwindcss from '@tailwindcss/vite';
import { siteFont, sitesTheme } from '@tonalisk-labs/kit/astro';

import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import process from 'node:process';

import site from '../marcoferra.config.json' with { type: 'json' };
import { datedSince } from './page-dates.mjs';

/** The site's config, whose meeting types the booking pages render. */
const MEETINGS = '../marcoferra.config.json';

/** The files a page is dated by: its route, and the data it renders.
 *  @param {string} pathname */
const sourcesOf = (pathname) => {
  const name = pathname.split('/').filter(Boolean).join('/');
  if (name === '') {
    return ['src/pages/index.astro'];
  }
  if (name === 'book' || name === 'work-with-me') {
    return [`src/pages/${name === 'book' ? 'book/index' : name}.astro`, MEETINGS];
  }
  return name.startsWith('book/')
    ? ['src/pages/book/[slug].astro', MEETINGS]
    : [`src/pages/${name}.astro`];
};

/** Astro's and Vite's caches, in a temp dir removed on exit, so a build leaves none behind. */
const cacheDir = mkdtempSync(`${tmpdir()}/astro-cache-`);
process.once('exit', () => {
  rmSync(cacheDir, { recursive: true, force: true });
});

/** 'system' for each platform's own, or one of kit's font families. */
const FONT = 'system';

// https://astro.build/config
export default defineConfig({
  site: 'https://www.marcoferra.com',

  integrations: [
    siteFont(FONT),
    react({ compiler: true }),
    // Cancelling and a paid booking's confirmation are reached from a link alone, and noindex.
    // Each page is dated by its sources, or by page-dates.json while the root commit is their last
    // change.
    sitemap({
      filter: (page) => !/\/book\/(?:cancel|[^/]+\/done)\/$/u.test(page),
      serialize: datedSince(import.meta.dirname, sourcesOf),
    }),
    sitesTheme({ theme: 'default', userThemes: true }),
  ],

  cacheDir,

  // Lossless: Astro 7's 'jsx' default drops line breaks before inline elements, and with them the
  // spaces between words and links or buttons.
  compressHTML: true,

  vite: {
    cacheDir: `${cacheDir}/vite`,
    plugins: [tailwindcss()],
    // In development only: the local backend, which serves the API without its /api prefix.
    server: {
      proxy: {
        '/api': {
          target: `http://${site.network.bind_addr}`,
          rewrite: (path) => path.replace(/^\/api/u, ''),
        },
      },
    },
  },

  // No code blocks, and Shiki's inline styles would need 'unsafe-inline' under the CSP.
  markdown: { syntaxHighlight: false },

  // Astro adds script-src and style-src hashes, and the rest falls back to 'self'.
  security: {
    csp: {
      directives: [
        "default-src 'self'",
        "object-src 'none'",
        "base-uri 'none'",
        "form-action 'self'",
      ],
    },
  },
});
