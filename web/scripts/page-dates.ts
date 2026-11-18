// Writes page-dates.json once, as the repository becomes one root commit: each sitemap page's
// lastmod from a build on the full history, which kit's lastChanged dated by the page's sources.
//   mise run build && node scripts/page-dates.ts   (in web/, on the history before the root)
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const WEB = path.resolve(import.meta.dirname, '..');
const DIST = path.join(WEB, 'dist');
const OUT = path.join(WEB, 'page-dates.json');

/** The text of every `<tag>` in `xml`, in order. */
const textsOf = (xml: string, tag: string): string[] =>
  [...xml.matchAll(new RegExp(`<${tag}>(?<text>[^<]*)</${tag}>`, 'gu'))].map(
    ({ groups: { text = '' } = {} }) => text,
  );

/** The site's sitemaps, as its index names them, by their file in the build. */
const sitemaps = (): string[] => {
  const index = readFileSync(path.join(DIST, 'sitemap-index.xml'), 'utf8');
  return textsOf(index, 'loc').map((loc) => path.join(DIST, new URL(loc).pathname));
};

/** Every page of a sitemap with its lastmod. */
const datesIn = (file: string): [string, string][] => {
  const xml = readFileSync(file, 'utf8');
  return [...xml.matchAll(/<url>(?<entry>.*?)<\/url>/gsu)].map(
    ({ groups: { entry = '' } = {} }) => {
      const [loc = ''] = textsOf(entry, 'loc');
      const [lastmod = ''] = textsOf(entry, 'lastmod');
      if (lastmod === '') {
        throw new Error(`${loc} has no lastmod in ${file}`);
      }
      return [loc, lastmod];
    },
  );
};

const dates = sitemaps().flatMap((file) => datesIn(file));
dates.sort(([one], [other]) => one.localeCompare(other));
writeFileSync(OUT, `${JSON.stringify(Object.fromEntries(dates), undefined, 2)}\n`);
console.log(`page-dates: ${String(dates.length)} pages written to ${path.relative(WEB, OUT)}`);
