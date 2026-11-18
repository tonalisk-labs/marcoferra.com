// A social card as a page of its own, 1200 × 630 in Inter, and the hash of what it's drawn from.
// cards.lock.json holds each committed card's hash, so a stale card fails its test.
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import path from 'node:path';

import { CARDS, type Card } from '../src/lib/cards.ts';

export const ROOT = path.resolve(import.meta.dirname, '..');
export const LOCK = path.join(import.meta.dirname, 'cards.lock.json');
export const SIZE = { width: 1200, height: 630 };

const face = (weight: number): string => {
  const font = readFileSync(
    path.join(ROOT, 'src', 'assets', 'fonts', `inter-latin-${weight.toString()}-normal.woff2`),
  );
  return `@font-face { font-family: Inter; font-weight: ${weight.toString()}; src: url(data:font/woff2;base64,${font.toString('base64')}); }`;
};

const FACES = [400, 600, 700].map((weight) => face(weight)).join('\n');

export const pageOf = ({ title, details }: Card): string => `<!doctype html>
<html lang="en-GB">
<meta charset="utf-8">
<style>
${FACES}
body { margin: 0; width: 1200px; height: 630px; box-sizing: border-box; display: flex; flex-direction: column; padding: 86px 96px 89px 97px; border-left: 28px solid #1d4ed8; background: #fff; color: #475569; font-family: Inter; }
b { font-size: 36px; line-height: 44px; font-weight: 600; color: #1d4ed8; }
h1 { margin: 30px 0 0; max-width: 780px; font-size: 66px; line-height: 75px; letter-spacing: -0.025em; color: #0f172a; }
p { margin: 35px 0 0; font-size: 30px; line-height: 36px; }
footer { margin-top: auto; font-size: 26px; font-weight: 600; }
</style>
<b>Marco Ferra</b>
<h1>${title}</h1>
<p>${details.join(' · ')}</p>
<footer>www.marcoferra.com</footer>
</html>
`;

/** The sources' hash, not the pixels', which Chromium draws differently on each platform. */
const hashOf = (card: Card): string => createHash('sha256').update(pageOf(card)).digest('hex');

export const hashes = (): Record<string, string> =>
  Object.fromEntries(Object.entries(CARDS).map(([name, card]) => [name, hashOf(card)]));
