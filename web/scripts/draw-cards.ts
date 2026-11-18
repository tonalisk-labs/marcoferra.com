// Draws every social card into public/og/ with Chromium, and records each one's source hash.
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

import { chromium } from '@playwright/test';

import { CARDS } from '../src/lib/cards.ts';
import { hashes, LOCK, pageOf, ROOT, SIZE } from './cards.ts';

const browser = await chromium.launch();
try {
  const page = await browser.newPage({ viewport: SIZE, deviceScaleFactor: 1 });
  for (const [name, card] of Object.entries(CARDS)) {
    // oxlint-disable-next-line no-await-in-loop -- one page draws the cards in turn.
    await page.setContent(pageOf(card));
    // oxlint-disable-next-line no-await-in-loop
    await page.evaluate(async () => {
      await globalThis.document.fonts.ready;
    });
    // oxlint-disable-next-line no-await-in-loop
    await page.screenshot({ path: path.join(ROOT, 'public', 'og', `${name}.png`) });
  }
} finally {
  await browser.close();
}
await writeFile(LOCK, `${JSON.stringify(hashes(), undefined, 2)}\n`);
console.log(`draw-cards: drew ${Object.keys(CARDS).length.toString()} cards`);
